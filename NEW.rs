    /// [`MessageMux`] implementation that start a new actor for each message.
    #[derive(Debug)]
    pub struct ActorStarter<R, NA, Arg> {
        runtime_ref: R,
        new_actor: NA,
        _phantom: PhantomData<Arg>,
    }

    impl<R, NA, Arg> ActorStarter<R, NA, Arg>
    where
        R: RuntimeAccess,
        NA: NewActor<Argument = Arg>,
        NA::Error: fmt::Display,
    {
        const fn new(runtime_ref: R, new_actor: NA) -> ActorStarter<R, NA, Arg> {
            ActorStarter {
                runtime_ref,
                new_actor,
                _phantom: PhantomData,
            }
        }
    }

    impl<'a, R, M, NA, Arg> MessageMux<'a, M> for ActorStarter<R, NA, Arg>
    where
        R: RuntimeAccess,
        NA: NewActor<Argument = Arg>,
        NA::Error: fmt::Display + Unpin + 'static,
        M: Into<Arg>,
        M: 'static + Unpin, // FIXME: remove.
    {
        type Error = NA::Error;
        type Future = StartActor<NA::Error>;

        fn route(&'a mut self, msg: M) -> Self::Future {
            todo!()
        }
    }

    #[derive(Debug)]
    pub struct StartActor<E> {
        err: Option<E>,
    }

    impl<E> Future for StartActor<E>
    where
        E: Unpin,
    {
        type Output = Result<(), E>;

        fn poll(self: Pin<&mut Self>, _ctx: &mut task::Context<'_>) -> Poll<Self::Output> {
            let res = match Pin::into_inner(self).err.take() {
                Some(err) => Err(err),
                None => Ok(()),
            };
            Poll::Ready(res)
        }
    }

# TODO.

Relay message to the correct actor based on some key. Keep actor refs in a
hashmap, if not in the hash map start a new actor for the message. Called `Mux`?

Use case: consensus actors of Stored.
