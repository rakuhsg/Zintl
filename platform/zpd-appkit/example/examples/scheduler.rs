use std::collections::VecDeque;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use zpd_appkit::runloop::{Application, ApplicationDelegate, RunLoopScheduler};
use zpd_appkit::ui::{CommandItem, CommandModifier, CommandRole, CommandSet, WindowAppMenu};

type MainTask = Box<dyn FnOnce() + Send + 'static>;
type MainTaskQueue = Arc<Mutex<VecDeque<MainTask>>>;

struct Delegate {
    tasks: MainTaskQueue,
}

impl ApplicationDelegate for Delegate {
    fn perform(&mut self) {
        let tasks: Vec<_> = self
            .tasks
            .lock()
            .expect("main task queue was poisoned")
            .drain(..)
            .collect();

        // Do not hold the queue lock while running user code. A task may
        // schedule more work from another thread.
        for task in tasks {
            task();
        }
    }
}

#[derive(Clone)]
struct MainTaskSender {
    tasks: MainTaskQueue,
    scheduler: RunLoopScheduler,
}

impl MainTaskSender {
    fn schedule(&self, task: impl FnOnce() + Send + 'static) -> bool {
        self.tasks
            .lock()
            .expect("main task queue was poisoned")
            .push_back(Box::new(task));
        self.scheduler.schedule()
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let tasks = MainTaskQueue::default();
    let application = Application::new(Delegate {
        tasks: tasks.clone(),
    })?;
    application.set_commands(&commands(), |_| {})?;
    let sender = MainTaskSender {
        tasks,
        scheduler: application.scheduler(),
    };

    let window = application.create_window(())?;
    window.set_size(480.0, 300.0)?;
    window.show()?;

    let main_thread = thread::current().id();
    let worker = thread::spawn(move || {
        for index in 1..=3 {
            thread::sleep(Duration::from_millis(500));
            let scheduled = sender.schedule(move || {
                assert_eq!(thread::current().id(), main_thread);
                println!("main-thread task {index} ran");
            });
            if !scheduled {
                break;
            }
        }
    });

    application.run();
    worker.join().expect("worker thread panicked");
    Ok(())
}

fn commands() -> CommandSet {
    CommandSet {
        app_menu: Some(WindowAppMenu {
            items: vec![CommandItem {
                id: None,
                title: "Quit Scheduler Example".into(),
                role: Some(CommandRole::Quit),
                key: Some("q".into()),
                modifiers: vec![CommandModifier::Cmd],
                enabled: true,
            }],
        }),
        menus: Vec::new(),
    }
}
