use zintl_native::{Context, MessageHandler, PlatformMessageLoop};

enum Message {}

struct Handler {}

impl MessageHandler<Message> for Handler {
    fn on_init(&mut self, _cx: impl Context<Message>) {
        println!("hello, world!");
    }
}

fn main() {
    let handler = Handler {};
    let m = PlatformMessageLoop::new(handler);
    m.run();
}
