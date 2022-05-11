use std::{
    any::TypeId,
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

use unsafe_unwrap::UnsafeUnwrap;

/// Panicy Read/Write Lock
///
/// Behaves the same as a normal RwLock, except the handles to the interior data have no lifetime.
/// The consequence of this is that if the lock is dropped before any handles, a panic will occur.
#[derive(Debug)]
pub struct PrwLock<T>(Arc<PrwLockInner<T>>);

#[derive(Debug)]
pub struct PrwLockInner<T> {
    data: T,
    access_state: AtomicU32,
}

#[derive(Debug)]
pub struct PrwReadLock<T>(Arc<PrwLockInner<T>>);

#[derive(Debug)]
pub struct PrwWriteLock<T>(Arc<PrwLockInner<T>>);

impl<T> PrwLock<T> {
    #[inline]
    pub fn new(data: T) -> Self {
        Self(Arc::new(PrwLockInner {
            data,
            access_state: AtomicU32::new(0),
        }))
    }

    #[inline]
    pub fn read(&self) -> PrwReadLock<T> {
        // See who is accessing the PrwLock
        // let access_state = self.0.access_state.fetch_add(1, Ordering::SeqCst);

        // Panic if there is a writer
        // assert_ne!(access_state, u32::MAX);

        PrwReadLock(self.0.clone())
    }

    #[inline]
    pub fn write(&self) -> PrwWriteLock<T> {
        // See who is accessing the PrwLock
        // let access_state = self.0.access_state.fetch_add(u32::MAX, Ordering::SeqCst);

        // Panic if there are readers or writers
        // assert_eq!(access_state, 0);

        PrwWriteLock(self.0.clone())
    }
}

impl<T> Drop for PrwLockInner<T> {
    #[inline]
    fn drop(&mut self) {
        // Panic if there are any outstanding access handles
        if self.access_state.load(Ordering::SeqCst) > 0 {
            panic!("outstanding access handle in archetype storage on drop");
        }
    }
}

unsafe impl<T> Send for PrwLockInner<T> {}

unsafe impl<T> Sync for PrwLockInner<T> {}

impl<T> Drop for PrwReadLock<T> {
    #[inline]
    fn drop(&mut self) {
        // self.0.access_state.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<T> Deref for PrwReadLock<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0.data
    }
}

unsafe impl<T> Send for PrwReadLock<T> {}

unsafe impl<T> Sync for PrwReadLock<T> {}

impl<T> Drop for PrwWriteLock<T> {
    #[inline]
    fn drop(&mut self) {
        // self.0.access_state.fetch_sub(u32::MAX, Ordering::SeqCst);
    }
}

impl<T> Deref for PrwWriteLock<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0.data
    }
}

impl<T> DerefMut for PrwWriteLock<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safe to unwrap because the PrwWriteLock guarantees there are no other references
        unsafe {
            (&self.0.data as *const T as *mut T)
                .as_mut()
                .unsafe_unwrap()
        }
    }
}

unsafe impl<T> Send for PrwWriteLock<T> {}

unsafe impl<T> Sync for PrwWriteLock<T> {}

#[cfg(test)]
mod tests {
    use super::PrwLock;

    #[test]
    fn prw_lock_test() {
        let lock = PrwLock::new(42);

        let handle1 = lock.read();
        assert_eq!(*handle1, 42);

        let handle2 = lock.read();
        assert_eq!(*handle2, 42);

        std::mem::drop(handle1);
        std::mem::drop(handle2);

        let mut handle3 = lock.write();
        assert_eq!(*handle3, 42);

        *handle3 += 27;

        assert_eq!(*handle3, 69);
    }

    #[test]
    #[should_panic]
    fn prw_lock_multiple_writers() {
        let lock = PrwLock::new(42);
        let mut _handle1 = lock.write();
        let mut _handle2 = lock.write();
    }

    #[test]
    #[should_panic]
    fn prw_lock_readers_and_writers() {
        let lock = PrwLock::new(42);
        let mut _handle1 = lock.read();
        let mut _handle2 = lock.write();
    }
}
