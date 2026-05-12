use std::sync::{Arc, LockResult, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Debug, Clone)]
pub enum MainActorError {
    NotInMainThread,
    LockError,
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
}
