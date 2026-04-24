use std::any::Any;

/// A unique identifier for an actor within a [`Stage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorRef(pub u64);

/// Contextual information for a message being processed.
#[derive(Debug, Clone, Copy)]
pub struct MsgCtx {
    /// The reference of the sender actor, if any.
    pub from: Option<ActorRef>,
    /// The reference of the recipient actor (the current actor).
    pub to: ActorRef,
}

pub type ActorMessage = (Box<dyn Any + Send>, MsgCtx);
