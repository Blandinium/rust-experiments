use std::any::Any;

#[derive(Debug, Clone, Copy)]
pub struct ActorRef(pub u64);

#[derive(Debug, Clone, Copy)]
pub struct MsgCtx {
    pub from: ActorRef,
    pub to: ActorRef,
}

pub trait Actor<T, M>
where
    M: Any,
{
    fn consume_message(&self, msg: Box<M>, ctx: MsgCtx, state: T) -> T;
    fn init(&self) -> T;
    fn consume_any_message(&self, msg: Box<dyn Any>, ctx: MsgCtx, state: T) -> T {
        let msg = msg
            .downcast::<M>()
            .expect("invalid message type for actor");
        self.consume_message(msg, ctx, state)
    }
}
