use crate::{Error, Result};
use tokio::sync::oneshot;
use tracing::warn;

#[derive(Debug)]
pub struct Transaction<Q, A> {
    pub req: Q,
    tx: oneshot::Sender<Result<A>>,
}

impl<Q, A> Transaction<Q, A> {
    /// Creates a new transaction pair: the transaction object to send,
    /// and the receiver to await the reply.
    pub fn new(req: Q) -> (Self, oneshot::Receiver<Result<A>>) {
        let (tx, rx) = oneshot::channel();
        (Transaction { req, tx }, rx)
    }

    fn send(self, reply: Result<A>) {
        if let Err(_) = self.tx.send(reply) {
            warn!("requestor closed channel")
        }
    }

    /// Consumes the transaction to send a successful reply.
    pub fn ok(self, val: A) {
        self.send(Ok(val))
    }

    /// Consumes the transaction to send an error reply.
    pub fn err(self, err: Error) {
        self.send(Err(err))
    }

    pub fn reply(self, reply: Result<A>) {
        self.send(reply)
    }
}
