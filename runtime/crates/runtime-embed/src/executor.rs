use crate::RuntimeError;
use crate::task::{Cancellation, RuntimeTask, TaskCompletion};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send + 'static>;

pub(crate) struct HostExecutor {
    sender: Mutex<Option<SyncSender<Job>>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl HostExecutor {
    pub(crate) fn new(worker_count: usize, queue_limit: usize) -> Result<Self, RuntimeError> {
        if worker_count == 0 || queue_limit == 0 {
            return Err(RuntimeError::InvalidConfiguration);
        }
        let (sender, receiver) = mpsc::sync_channel::<Job>(queue_limit);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let receiver = receiver.clone();
            let worker = thread::Builder::new()
                .name(format!("runtime-embed-host-{index}"))
                .spawn(move || worker_main(&receiver))
                .map_err(|_| RuntimeError::Internal)?;
            workers.push(worker);
        }
        Ok(Self {
            sender: Mutex::new(Some(sender)),
            workers: Mutex::new(workers),
        })
    }

    pub(crate) fn submit<T: Send + 'static>(
        &self,
        operation: impl FnOnce(Cancellation) -> Result<T, RuntimeError> + Send + 'static,
    ) -> Result<RuntimeTask<T>, RuntimeError> {
        let (task, completion, cancellation) = RuntimeTask::pair();
        let job = Box::new(move || run_job(operation, cancellation, completion));
        let sender = self.sender.lock().map_err(|_| RuntimeError::Internal)?;
        let sender = sender.as_ref().ok_or(RuntimeError::ShuttingDown)?;
        sender.try_send(job).map_err(|error| match error {
            TrySendError::Full(_) => RuntimeError::QuotaExceeded,
            TrySendError::Disconnected(_) => RuntimeError::ShuttingDown,
        })?;
        Ok(task)
    }

    pub(crate) fn submit_with_timeout<T: Send + 'static>(
        &self,
        timeout: Duration,
        operation: impl FnOnce(Cancellation) -> Result<T, RuntimeError> + Send + 'static,
    ) -> Result<RuntimeTask<T>, RuntimeError> {
        if timeout.is_zero() {
            return Err(RuntimeError::InvalidConfiguration);
        }
        let (task, completion, cancellation) = RuntimeTask::pair();
        let timeout_completion = completion.clone();
        let timeout_cancellation = cancellation.clone();
        let timer = thread::Builder::new()
            .name("runtime-embed-timeout".to_string())
            .spawn(move || timeout_completion.timeout(timeout, &timeout_cancellation))
            .map_err(|_| RuntimeError::Internal)?;
        completion.install_timer(timer)?;
        let job = Box::new(move || run_job(operation, cancellation, completion));
        let sender = self.sender.lock().map_err(|_| RuntimeError::Internal)?;
        let sender = sender.as_ref().ok_or(RuntimeError::ShuttingDown)?;
        sender.try_send(job).map_err(|error| match error {
            TrySendError::Full(_) => RuntimeError::QuotaExceeded,
            TrySendError::Disconnected(_) => RuntimeError::ShuttingDown,
        })?;
        Ok(task)
    }

    pub(crate) fn shutdown(&self) -> Result<(), RuntimeError> {
        self.sender
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .take();
        let mut workers = self.workers.lock().map_err(|_| RuntimeError::Internal)?;
        for worker in workers.drain(..) {
            worker.join().map_err(|_| RuntimeError::Internal)?;
        }
        Ok(())
    }
}

impl Drop for HostExecutor {
    fn drop(&mut self) {
        if let Ok(sender) = self.sender.get_mut() {
            sender.take();
        }
        if let Ok(workers) = self.workers.get_mut() {
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
        }
    }
}

fn run_job<T>(
    operation: impl FnOnce(Cancellation) -> Result<T, RuntimeError>,
    cancellation: Cancellation,
    completion: TaskCompletion<T>,
) {
    if cancellation.is_cancelled() {
        completion.complete(Err(RuntimeError::Cancelled));
        return;
    }
    completion.complete(operation(cancellation));
}

fn worker_main(receiver: &Mutex<Receiver<Job>>) {
    loop {
        let job = {
            let Ok(receiver) = receiver.lock() else {
                return;
            };
            receiver.recv()
        };
        let Ok(job) = job else {
            return;
        };
        job();
    }
}
