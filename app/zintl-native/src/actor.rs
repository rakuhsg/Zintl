use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};

#[derive(Debug, Clone)]
pub enum MainActorError {
    NotInMainThread,
    LockError,
    Dropped,
}

pub type MainActorResult<T> = Result<T, MainActorError>;

fn lock_result<T>(val: LockResult<T>) -> MainActorResult<T> {
    match val {
        LockResult::Ok(v) => Ok(v),
        LockResult::Err(..) => Err(MainActorError::LockError),
    }
}

#[derive(Clone, Copy)]
pub struct MainMarker(std::marker::PhantomData<std::sync::MutexGuard<'static, ()>>);

impl MainMarker {
    pub(crate) fn new() -> Self {
        MainMarker(std::marker::PhantomData)
    }
}

pub struct MainActor<T> {
    inner: Arc<RwLock<T>>,
}

pub struct MainActorRef<T> {
    inner: Weak<RwLock<T>>,
}

impl<T> Clone for MainActorRef<T> {
    fn clone(&self) -> Self {
        MainActorRef {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Clone for MainActor<T> {
    fn clone(&self) -> Self {
        MainActor {
            inner: self.inner.clone(),
        }
    }
}

impl<T> MainActor<T> {
    pub fn new(_marker: MainMarker, value: T) -> Self {
        MainActor {
            inner: Arc::new(value.into()),
        }
    }

    pub fn read(&self, _marker: MainMarker) -> MainActorResult<RwLockReadGuard<'_, T>> {
        lock_result(self.inner.read())
    }

    pub fn write(&self, _marker: MainMarker) -> MainActorResult<RwLockWriteGuard<'_, T>> {
        lock_result(self.inner.write())
    }

    pub fn downgrade(&self) -> MainActorRef<T> {
        MainActorRef {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl<T> MainActorRef<T> {
    pub fn upgrade(&self) -> MainActorResult<MainActor<T>> {
        self.inner
            .upgrade()
            .map(|inner| MainActor { inner })
            .ok_or(MainActorError::Dropped)
    }
}
