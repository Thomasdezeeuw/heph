//! Collection of hacks.

use crate::rt::RuntimeRef;

/// A hack to allow us to call `Runtime<!>.run`.
pub trait SetupFn: Send + Clone + 'static {
    type Error: Send;

    fn setup(self, runtime_ref: RuntimeRef) -> Result<(), Self::Error>;
}

impl<F, E> SetupFn for F
where
    F: FnOnce(RuntimeRef) -> Result<(), E> + Send + Clone + 'static,
    E: Send,
{
    type Error = E;

    fn setup(self, runtime_ref: RuntimeRef) -> Result<(), Self::Error> {
        (self)(runtime_ref)
    }
}

impl SetupFn for ! {
    type Error = !;

    fn setup(self, _: RuntimeRef) -> Result<(), Self::Error> {
        self
    }
}
