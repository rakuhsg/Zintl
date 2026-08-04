//! macOS kqueue readiness backend. Unsafe code is confined to syscall wrappers.

use reactor_api::{EventFlags, Interest, Reactor, ReactorError, ReactorEvent, SourceRef};
use std::collections::HashMap;
use std::ffi::c_void;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

const WAKER_IDENT: usize = 1;
const MAX_EVENTS: usize = 64;

struct Slot {
    generation: u32,
    source: Option<SourceRef>,
    interest: Interest,
    retired: bool,
}

/// Generational registration; it contains no descriptor.
#[derive(Clone, Debug)]
pub struct Registration {
    slot: u32,
    generation: u32,
    source: SourceRef,
}

/// Owns the kqueue descriptor and backend-local source descriptors.
pub struct KqueueReactor {
    queue: OwnedFd,
    sources: HashMap<SourceRef, OwnedFd>,
    slots: Vec<Slot>,
}

enum DriverCommand {
    Register {
        source: SourceRef,
        interest: Interest,
        response: mpsc::Sender<Result<Registration, ReactorError>>,
    },
    Reregister {
        registration: Registration,
        interest: Interest,
        response: mpsc::Sender<Result<(), ReactorError>>,
    },
    Deregister {
        registration: Registration,
        response: mpsc::Sender<Result<(), ReactorError>>,
    },
    Shutdown,
}

/// Runs blocking `kevent` only on a dedicated background thread. Configuration
/// commands wake that thread; event draining is bounded and non-blocking.
pub struct KqueueDriver {
    commands: SyncSender<DriverCommand>,
    events: Receiver<ReactorEvent>,
    waker: OwnedFd,
    thread: Option<JoinHandle<()>>,
}

impl KqueueDriver {
    /// Moves a configured reactor onto its dedicated thread.
    ///
    /// # Errors
    ///
    /// Rejects zero queue limits or thread/waker creation failure.
    pub fn start(
        reactor: KqueueReactor,
        command_limit: usize,
        event_limit: usize,
    ) -> Result<Self, ReactorError> {
        if command_limit == 0 || event_limit == 0 {
            return Err(ReactorError::InvalidRegistration);
        }
        // SAFETY: `dup` receives a live kqueue descriptor and returns a new
        // independently owned reference to the same kernel queue or -1.
        let duplicate = unsafe { libc::dup(reactor.queue.as_raw_fd()) };
        if duplicate < 0 {
            return Err(ReactorError::Backend);
        }
        // SAFETY: Successful `dup` returned a new descriptor owned exactly once.
        let waker = unsafe { OwnedFd::from_raw_fd(duplicate) };
        set_close_on_exec(&waker)?;
        let (command_sender, command_receiver) = mpsc::sync_channel(command_limit);
        let (event_sender, event_receiver) = mpsc::sync_channel(event_limit);
        let thread = thread::Builder::new()
            .name("runtime-kqueue-reactor".to_owned())
            .spawn(move || driver_main(reactor, &command_receiver, &event_sender))
            .map_err(|_| ReactorError::Backend)?;
        Ok(Self {
            commands: command_sender,
            events: event_receiver,
            waker,
            thread: Some(thread),
        })
    }

    /// Registers on the reactor thread. This waits only for the woken command,
    /// never performs a caller-thread readiness poll, and is intended for a
    /// non-main runtime executor.
    ///
    /// # Errors
    ///
    /// Returns an error for command backpressure, driver shutdown, or backend
    /// registration failure.
    pub fn register(
        &self,
        source: SourceRef,
        interest: Interest,
    ) -> Result<Registration, ReactorError> {
        let (sender, receiver) = mpsc::channel();
        self.send(DriverCommand::Register {
            source,
            interest,
            response: sender,
        })?;
        receiver.recv().map_err(|_| ReactorError::Backend)?
    }

    /// Changes interest on the reactor thread.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale registration, command backpressure, driver
    /// shutdown, or backend failure.
    pub fn reregister(
        &self,
        registration: Registration,
        interest: Interest,
    ) -> Result<(), ReactorError> {
        let (sender, receiver) = mpsc::channel();
        self.send(DriverCommand::Reregister {
            registration,
            interest,
            response: sender,
        })?;
        receiver.recv().map_err(|_| ReactorError::Backend)?
    }

    /// Deregisters on the reactor thread.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale registration, command backpressure, driver
    /// shutdown, or backend failure.
    pub fn deregister(&self, registration: Registration) -> Result<(), ReactorError> {
        let (sender, receiver) = mpsc::channel();
        self.send(DriverCommand::Deregister {
            registration,
            response: sender,
        })?;
        receiver.recv().map_err(|_| ReactorError::Backend)?
    }

    /// Drains at most `maximum` events without waiting.
    pub fn drain_events(&self, maximum: usize, output: &mut Vec<ReactorEvent>) {
        for _ in 0..maximum {
            let Ok(event) = self.events.try_recv() else {
                break;
            };
            output.push(event);
        }
    }

    /// Stops and joins the dedicated thread. Embedders call this from their
    /// shutdown executor, never `MainActor`.
    pub fn shutdown_blocking_non_main(&mut self) {
        if self.thread.is_none() {
            return;
        }
        let _ = trigger_waker(self.waker.as_raw_fd());
        let _ = self.commands.send(DriverCommand::Shutdown);
        let _ = trigger_waker(self.waker.as_raw_fd());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn send(&self, command: DriverCommand) -> Result<(), ReactorError> {
        match self.commands.try_send(command) {
            Ok(()) => trigger_waker(self.waker.as_raw_fd()),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                Err(ReactorError::Backend)
            }
        }
    }

    #[cfg(test)]
    fn recv_event_timeout(&self, timeout: std::time::Duration) -> Option<ReactorEvent> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for KqueueDriver {
    fn drop(&mut self) {
        self.shutdown_blocking_non_main();
    }
}

impl KqueueReactor {
    /// Creates a close-on-exec kqueue and installs a coalescible user wake event.
    ///
    /// # Errors
    ///
    /// Returns a backend error if queue creation, close-on-exec, or wake event
    /// installation fails. Partially created descriptors are closed by RAII.
    pub fn new() -> Result<Self, ReactorError> {
        // SAFETY: `kqueue` takes no pointers and returns a newly owned descriptor
        // or -1. Ownership is transferred once into `OwnedFd`.
        let descriptor = unsafe { libc::kqueue() };
        if descriptor < 0 {
            return Err(ReactorError::Backend);
        }
        // SAFETY: `descriptor` is a unique non-negative descriptor returned by
        // `kqueue` and has not been wrapped or closed yet.
        let queue = unsafe { OwnedFd::from_raw_fd(descriptor) };
        set_close_on_exec(&queue)?;
        let reactor = Self {
            queue,
            sources: HashMap::new(),
            slots: Vec::new(),
        };
        reactor.install_waker()?;
        Ok(reactor)
    }

    /// Transfers a safely owned descriptor into the backend under an opaque source ID.
    ///
    /// # Errors
    ///
    /// Rejects duplicate source identities or failure to set close-on-exec.
    pub fn attach_source(
        &mut self,
        source: SourceRef,
        descriptor: OwnedFd,
    ) -> Result<(), ReactorError> {
        if self.sources.contains_key(&source) {
            return Err(ReactorError::InvalidRegistration);
        }
        set_close_on_exec(&descriptor)?;
        self.sources.insert(source, descriptor);
        Ok(())
    }

    /// Drops and closes an unattached source descriptor.
    ///
    /// # Errors
    ///
    /// Rejects unknown sources or removal while a live registration exists.
    pub fn remove_source(&mut self, source: SourceRef) -> Result<(), ReactorError> {
        if self
            .slots
            .iter()
            .any(|slot| slot.source == Some(source) && !slot.retired)
        {
            return Err(ReactorError::InvalidRegistration);
        }
        self.sources
            .remove(&source)
            .map(drop)
            .ok_or(ReactorError::InvalidRegistration)
    }

    fn install_waker(&self) -> Result<(), ReactorError> {
        let change = event(
            WAKER_IDENT,
            libc::EVFILT_USER,
            libc::EV_ADD | libc::EV_CLEAR,
            0,
            0,
        );
        submit_changes(self.queue.as_raw_fd(), &[change])
    }

    fn validate_registration(&self, registration: &Registration) -> Result<&Slot, ReactorError> {
        let slot = self
            .slots
            .get(registration.slot as usize)
            .ok_or(ReactorError::InvalidRegistration)?;
        if slot.retired
            || slot.generation != registration.generation
            || slot.source != Some(registration.source)
        {
            return Err(ReactorError::InvalidRegistration);
        }
        Ok(slot)
    }

    fn decode_event(&self, item: &libc::kevent) -> Option<ReactorEvent> {
        if item.filter == libc::EVFILT_USER && item.ident == WAKER_IDENT {
            return None;
        }
        let token = item.udata as usize as u64;
        let (slot_index, generation) = unpack_token(token);
        let slot = self.slots.get(slot_index as usize)?;
        if slot.retired || slot.generation != generation {
            return None;
        }
        let source = slot.source?;
        let mut flags = EventFlags::empty();
        if item.filter == libc::EVFILT_READ {
            flags = flags.union(EventFlags::READABLE);
        }
        if item.filter == libc::EVFILT_WRITE {
            flags = flags.union(EventFlags::WRITABLE);
        }
        if item.flags & libc::EV_ERROR != 0 {
            flags = flags.union(EventFlags::ERROR);
        }
        if item.flags & libc::EV_EOF != 0 {
            flags = flags.union(EventFlags::HANGUP);
        }
        Some(ReactorEvent { source, flags })
    }
}

impl Reactor for KqueueReactor {
    type Registration = Registration;

    fn register(
        &mut self,
        source: SourceRef,
        interest: Interest,
    ) -> Result<Self::Registration, ReactorError> {
        if !interest.readable && !interest.writable {
            return Err(ReactorError::InvalidRegistration);
        }
        let descriptor = self
            .sources
            .get(&source)
            .ok_or(ReactorError::InvalidRegistration)?
            .as_raw_fd();
        if self
            .slots
            .iter()
            .any(|slot| slot.source == Some(source) && !slot.retired)
        {
            return Err(ReactorError::InvalidRegistration);
        }
        let slot_index = self
            .slots
            .iter()
            .position(|slot| slot.source.is_none() && !slot.retired)
            .unwrap_or_else(|| {
                self.slots.push(Slot {
                    generation: 1,
                    source: None,
                    interest,
                    retired: false,
                });
                self.slots.len() - 1
            });
        let slot_u32 = u32::try_from(slot_index).map_err(|_| ReactorError::Backend)?;
        let generation = self.slots[slot_index].generation;
        let token = pack_token(slot_u32, generation);
        let changes = interest_changes(
            descriptor,
            Interest {
                readable: false,
                writable: false,
            },
            interest,
            token,
        );
        submit_changes(self.queue.as_raw_fd(), &changes)?;
        let slot = &mut self.slots[slot_index];
        slot.source = Some(source);
        slot.interest = interest;
        Ok(Registration {
            slot: slot_u32,
            generation,
            source,
        })
    }

    fn reregister(
        &mut self,
        registration: &Self::Registration,
        interest: Interest,
    ) -> Result<(), ReactorError> {
        if !interest.readable && !interest.writable {
            return Err(ReactorError::InvalidRegistration);
        }
        let old_interest = self.validate_registration(registration)?.interest;
        let descriptor = self
            .sources
            .get(&registration.source)
            .ok_or(ReactorError::InvalidRegistration)?
            .as_raw_fd();
        let changes = interest_changes(
            descriptor,
            old_interest,
            interest,
            pack_token(registration.slot, registration.generation),
        );
        submit_changes(self.queue.as_raw_fd(), &changes)?;
        self.slots[registration.slot as usize].interest = interest;
        Ok(())
    }

    fn deregister(&mut self, registration: Self::Registration) -> Result<(), ReactorError> {
        let interest = self.validate_registration(&registration)?.interest;
        let descriptor = self
            .sources
            .get(&registration.source)
            .ok_or(ReactorError::InvalidRegistration)?
            .as_raw_fd();
        let changes = delete_changes(descriptor, interest);
        submit_changes(self.queue.as_raw_fd(), &changes)?;
        let slot = &mut self.slots[registration.slot as usize];
        slot.source = None;
        match slot.generation.checked_add(1) {
            Some(generation) => slot.generation = generation,
            None => slot.retired = true,
        }
        Ok(())
    }

    fn poll(
        &mut self,
        deadline: Option<Instant>,
        output: &mut Vec<ReactorEvent>,
    ) -> Result<(), ReactorError> {
        loop {
            let timeout = deadline.map(deadline_timeout);
            let timeout_pointer = timeout.as_ref().map_or(ptr::null(), ptr::from_ref);
            let mut events = [empty_event(); MAX_EVENTS];
            let event_capacity = i32::try_from(MAX_EVENTS).map_err(|_| ReactorError::Backend)?;
            // SAFETY: The queue descriptor is live; change list is null with
            // count zero; `events` is fully initialized, writable, correctly
            // aligned, and has `MAX_EVENTS` elements; timeout is null or points
            // to a live `timespec` for the duration of the call.
            let count = unsafe {
                libc::kevent(
                    self.queue.as_raw_fd(),
                    ptr::null(),
                    0,
                    events.as_mut_ptr(),
                    event_capacity,
                    timeout_pointer,
                )
            };
            if count < 0 {
                if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(ReactorError::Backend);
            }
            let count = usize::try_from(count).map_err(|_| ReactorError::Backend)?;
            for item in events.iter().take(count) {
                if let Some(event) = self.decode_event(item) {
                    output.push(event);
                }
            }
            return Ok(());
        }
    }

    fn wake(&self) -> Result<(), ReactorError> {
        let change = event(WAKER_IDENT, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER, 0);
        submit_changes(self.queue.as_raw_fd(), &[change])
    }
}

fn set_close_on_exec(descriptor: &OwnedFd) -> Result<(), ReactorError> {
    // SAFETY: `descriptor` is live and `F_GETFD` has no pointer argument.
    let flags = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFD) };
    if flags < 0 {
        return Err(ReactorError::Backend);
    }
    // SAFETY: `descriptor` remains live and the third argument is the integer
    // flag set returned by `F_GETFD` with `FD_CLOEXEC` added.
    if unsafe {
        libc::fcntl(
            descriptor.as_raw_fd(),
            libc::F_SETFD,
            flags | libc::FD_CLOEXEC,
        )
    } < 0
    {
        return Err(ReactorError::Backend);
    }
    Ok(())
}

fn interest_changes(
    descriptor: i32,
    old: Interest,
    new: Interest,
    token: u64,
) -> Vec<libc::kevent> {
    let mut changes = Vec::with_capacity(2);
    filter_change(
        &mut changes,
        descriptor,
        libc::EVFILT_READ,
        old.readable,
        new.readable,
        token,
    );
    filter_change(
        &mut changes,
        descriptor,
        libc::EVFILT_WRITE,
        old.writable,
        new.writable,
        token,
    );
    changes
}

fn filter_change(
    changes: &mut Vec<libc::kevent>,
    descriptor: i32,
    filter: i16,
    old: bool,
    new: bool,
    token: u64,
) {
    if old == new {
        return;
    }
    let flags = if new {
        libc::EV_ADD | libc::EV_ENABLE
    } else {
        libc::EV_DELETE
    };
    changes.push(event(descriptor_ident(descriptor), filter, flags, 0, token));
}

fn delete_changes(descriptor: i32, interest: Interest) -> Vec<libc::kevent> {
    let mut changes = Vec::with_capacity(2);
    if interest.readable {
        changes.push(event(
            descriptor_ident(descriptor),
            libc::EVFILT_READ,
            libc::EV_DELETE,
            0,
            0,
        ));
    }
    if interest.writable {
        changes.push(event(
            descriptor_ident(descriptor),
            libc::EVFILT_WRITE,
            libc::EV_DELETE,
            0,
            0,
        ));
    }
    changes
}

fn submit_changes(queue: i32, changes: &[libc::kevent]) -> Result<(), ReactorError> {
    if changes.is_empty() {
        return Ok(());
    }
    // SAFETY: The queue descriptor is live for each caller; `changes` is a
    // correctly aligned initialized slice held for this call; no output or
    // timeout pointer is supplied.
    let result = unsafe {
        libc::kevent(
            queue,
            changes.as_ptr(),
            i32::try_from(changes.len()).map_err(|_| ReactorError::Backend)?,
            ptr::null_mut(),
            0,
            ptr::null(),
        )
    };
    if result < 0 {
        Err(ReactorError::Backend)
    } else {
        Ok(())
    }
}

fn trigger_waker(queue: i32) -> Result<(), ReactorError> {
    submit_changes(
        queue,
        &[event(
            WAKER_IDENT,
            libc::EVFILT_USER,
            0,
            libc::NOTE_TRIGGER,
            0,
        )],
    )
}

fn driver_main(
    mut reactor: KqueueReactor,
    commands: &Receiver<DriverCommand>,
    events: &SyncSender<ReactorEvent>,
) {
    let mut polled = Vec::new();
    loop {
        if reactor.poll(None, &mut polled).is_err() {
            return;
        }
        for event in polled.drain(..) {
            match events.try_send(event) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        while let Ok(command) = commands.try_recv() {
            match command {
                DriverCommand::Register {
                    source,
                    interest,
                    response,
                } => {
                    let _ = response.send(reactor.register(source, interest));
                }
                DriverCommand::Reregister {
                    registration,
                    interest,
                    response,
                } => {
                    let _ = response.send(reactor.reregister(&registration, interest));
                }
                DriverCommand::Deregister {
                    registration,
                    response,
                } => {
                    let _ = response.send(reactor.deregister(registration));
                }
                DriverCommand::Shutdown => return,
            }
        }
    }
}

fn empty_event() -> libc::kevent {
    event(0, 0, 0, 0, 0)
}

fn event(
    ident: usize,
    filter: i16,
    event_flags: u16,
    filter_flags: u32,
    token: u64,
) -> libc::kevent {
    libc::kevent {
        ident,
        filter,
        flags: event_flags,
        fflags: filter_flags,
        data: 0,
        udata: usize::try_from(token).unwrap_or_default() as *mut c_void,
    }
}

fn pack_token(slot: u32, generation: u32) -> u64 {
    (u64::from(generation) << 32) | (u64::from(slot) + 1)
}

fn unpack_token(token: u64) -> (u32, u32) {
    (
        u32::try_from(token & u64::from(u32::MAX))
            .unwrap_or_default()
            .wrapping_sub(1),
        u32::try_from(token >> 32).unwrap_or_default(),
    )
}

fn descriptor_ident(descriptor: i32) -> usize {
    usize::try_from(descriptor).unwrap_or_default()
}

fn deadline_timeout(deadline: Instant) -> libc::timespec {
    let duration = deadline.saturating_duration_since(Instant::now());
    libc::timespec {
        tv_sec: duration.as_secs().try_into().unwrap_or(libc::time_t::MAX),
        tv_nsec: duration.subsec_nanos().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{KqueueDriver, KqueueReactor, event, pack_token};
    use reactor_api::{
        EventFlags, Interest, Reactor, ReactorError, SourceRef, run_readable_conformance,
    };
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::time::{Duration, Instant};

    fn pipe() -> (OwnedFd, std::fs::File) {
        let mut descriptors = [-1; 2];
        // SAFETY: `descriptors` is aligned writable storage for two integers;
        // success initializes two uniquely owned descriptors.
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        // SAFETY: Successful `pipe` returned two distinct owned descriptors,
        // each wrapped exactly once.
        let read = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        // SAFETY: The write descriptor is distinct and transferred once.
        let write = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
        (read, write)
    }

    #[test]
    // Verifies pipe readability, timeout conversion, and source identity.
    fn pipe_readiness_is_reported() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(7);
        let (read, mut write) = pipe();
        reactor.attach_source(source, read).expect("attached");
        let registration = reactor
            .register(
                source,
                Interest {
                    readable: true,
                    writable: false,
                },
            )
            .expect("registered");
        write.write_all(&[1]).expect("write");
        let mut output = Vec::new();
        reactor
            .poll(Some(Instant::now() + Duration::from_secs(1)), &mut output)
            .expect("poll");
        assert_eq!(output[0].source, source);
        assert!(output[0].flags.contains(EventFlags::READABLE));
        reactor.deregister(registration).expect("deregistered");
        reactor.remove_source(source).expect("removed");
    }

    #[test]
    // Verifies kqueue passes the reusable readiness contract used by future reactor backends.
    fn reusable_reactor_conformance_harness_passes() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(70);
        let (read, mut write) = pipe();
        reactor.attach_source(source, read).expect("attached");
        run_readable_conformance(
            &mut reactor,
            source,
            || write.write_all(&[1]).map_err(|_| ReactorError::Backend),
            Instant::now() + Duration::from_secs(1),
        )
        .expect("conformance");
        reactor.remove_source(source).expect("removed");
    }

    #[test]
    // Verifies writable readiness uses the same portable registration contract.
    fn pipe_write_readiness_is_reported() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(14);
        let (read, write) = pipe();
        let write: OwnedFd = write.into();
        reactor.attach_source(source, write).expect("attached");
        let registration = reactor
            .register(
                source,
                Interest {
                    readable: false,
                    writable: true,
                },
            )
            .expect("registered");
        let mut output = Vec::new();
        reactor
            .poll(Some(Instant::now() + Duration::from_secs(1)), &mut output)
            .expect("poll");
        assert!(output[0].flags.contains(EventFlags::WRITABLE));
        reactor.deregister(registration).expect("deregistered");
        drop(read);
    }

    #[test]
    // Verifies wake interrupts a blocking deadline without producing fake I/O.
    fn wake_interrupts_poll() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        reactor.wake().expect("wake");
        let mut output = Vec::new();
        reactor
            .poll(Some(Instant::now() + Duration::from_secs(1)), &mut output)
            .expect("poll");
        assert!(output.is_empty());
    }

    #[test]
    // Verifies a consumed registration is stale after slot generation advances.
    fn stale_registration_is_rejected() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(7);
        let (read, _write) = pipe();
        reactor.attach_source(source, read).expect("attached");
        let registration = reactor
            .register(
                source,
                Interest {
                    readable: true,
                    writable: false,
                },
            )
            .expect("registered");
        let stale = registration.clone();
        reactor.deregister(registration).expect("deregistered");
        assert_eq!(
            reactor.reregister(
                &stale,
                Interest {
                    readable: true,
                    writable: false
                }
            ),
            Err(ReactorError::InvalidRegistration)
        );
    }

    #[test]
    // Verifies an elapsed deadline performs a non-blocking empty poll.
    fn elapsed_deadline_is_non_blocking() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let mut output = Vec::new();
        reactor
            .poll(Some(Instant::now()), &mut output)
            .expect("poll");
        assert!(output.is_empty());
    }

    #[test]
    // Verifies EOF is mapped to portable hangup/readable semantics.
    fn pipe_eof_is_reported_without_errno_exposure() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(8);
        let (read, write) = pipe();
        reactor.attach_source(source, read).expect("attached");
        let registration = reactor
            .register(
                source,
                Interest {
                    readable: true,
                    writable: false,
                },
            )
            .expect("registered");
        drop(write);
        let mut output = Vec::new();
        reactor
            .poll(Some(Instant::now() + Duration::from_secs(1)), &mut output)
            .expect("poll");
        assert!(output[0].flags.contains(EventFlags::HANGUP));
        reactor.deregister(registration).expect("deregistered");
    }

    #[test]
    // Verifies reregister and close cleanup do not deliver a stale token.
    fn reregister_then_close_race_cleans_up() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(9);
        let (read, write) = pipe();
        reactor.attach_source(source, read).expect("attached");
        let registration = reactor
            .register(
                source,
                Interest {
                    readable: true,
                    writable: false,
                },
            )
            .expect("registered");
        reactor
            .reregister(
                &registration,
                Interest {
                    readable: true,
                    writable: false,
                },
            )
            .expect("reregistered");
        reactor.deregister(registration).expect("deregistered");
        reactor.remove_source(source).expect("closed source");
        drop(write);
        let mut output = Vec::new();
        reactor
            .poll(Some(Instant::now()), &mut output)
            .expect("poll");
        assert!(output.is_empty());
    }

    #[test]
    // Verifies blocking kevent runs on the named driver thread, not the caller.
    fn driver_delivers_readiness_from_background_thread() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(10);
        let (read, mut write) = pipe();
        reactor.attach_source(source, read).expect("attached");
        let mut driver = KqueueDriver::start(reactor, 4, 4).expect("driver");
        let registration = driver
            .register(
                source,
                Interest {
                    readable: true,
                    writable: false,
                },
            )
            .expect("registered");
        write.write_all(&[1]).expect("write");
        let event = driver
            .recv_event_timeout(Duration::from_secs(1))
            .expect("event");
        assert_eq!(event.source, source);
        driver.deregister(registration).expect("deregistered");
        driver.shutdown_blocking_non_main();
    }

    #[test]
    // Verifies dropping the backend closes both queue and attached source descriptors.
    fn reactor_drop_closes_all_owned_descriptors() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        // Keep the probe descriptors outside the range concurrently used by
        // neighboring tests so numeric reuse cannot make this assertion flaky.
        // SAFETY: `F_DUPFD_CLOEXEC` reads a valid descriptor and returns a new
        // owned descriptor or `-1`; the result is checked before ownership.
        let queue_high =
            unsafe { libc::fcntl(reactor.queue.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10_000) };
        assert!(queue_high >= 10_000);
        // SAFETY: `queue_high` is a fresh descriptor now uniquely owned here.
        reactor.queue = unsafe { OwnedFd::from_raw_fd(queue_high) };
        let queue_descriptor = reactor.queue.as_raw_fd();
        let source = SourceRef::new(11);
        let (read, _write) = pipe();
        // SAFETY: Same checked descriptor duplication contract as above.
        let source_high = unsafe { libc::fcntl(read.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10_000) };
        assert!(source_high >= 10_000);
        // SAFETY: `source_high` is a fresh descriptor now uniquely owned here.
        let read = unsafe { OwnedFd::from_raw_fd(source_high) };
        let source_descriptor = read.as_raw_fd();
        reactor.attach_source(source, read).expect("attached");
        drop(reactor);
        // SAFETY: `F_GETFD` has no pointer argument. The test intentionally
        // probes the former numeric descriptors only to verify RAII closure.
        assert_eq!(unsafe { libc::fcntl(queue_descriptor, libc::F_GETFD) }, -1);
        // SAFETY: Same as above for the formerly attached source descriptor.
        assert_eq!(unsafe { libc::fcntl(source_descriptor, libc::F_GETFD) }, -1);
    }

    #[test]
    // Verifies kernel error flags map to portable semantics without exposing errno.
    fn kernel_error_event_maps_to_portable_error() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(12);
        let (read, _write) = pipe();
        reactor.attach_source(source, read).expect("attached");
        let registration = reactor
            .register(
                source,
                Interest {
                    readable: true,
                    writable: false,
                },
            )
            .expect("registered");
        let synthetic = event(
            0,
            libc::EVFILT_READ,
            libc::EV_ERROR,
            0,
            pack_token(registration.slot, registration.generation),
        );
        let decoded = reactor.decode_event(&synthetic).expect("decoded");
        assert!(decoded.flags.contains(EventFlags::ERROR));
    }

    #[test]
    // Verifies a queued event for a reused registration slot is discarded.
    fn stale_kernel_token_is_discarded() {
        let mut reactor = KqueueReactor::new().expect("reactor");
        let source = SourceRef::new(13);
        let (read, _write) = pipe();
        reactor.attach_source(source, read).expect("attached");
        let registration = reactor
            .register(
                source,
                Interest {
                    readable: true,
                    writable: false,
                },
            )
            .expect("registered");
        let synthetic = event(
            0,
            libc::EVFILT_READ,
            0,
            0,
            pack_token(registration.slot, registration.generation),
        );
        reactor.deregister(registration).expect("deregistered");
        assert!(reactor.decode_event(&synthetic).is_none());
    }
}
