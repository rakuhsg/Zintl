pub trait MessageHandler<M> {}

pub trait MessageLoop<M, H: MessageHandler<M>> {
    fn run(&mut self);
}
