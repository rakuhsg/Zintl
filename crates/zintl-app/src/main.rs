use zintl_native::{MessageHandler, MessageLoop, PlatformMessageLoop};

enum Message {}

struct Handler {}

impl MessageHandler<Message> for Handler {}

fn main() {
    let handler = Handler {};
    let mut m = PlatformMessageLoop::new(handler);
    m.run();
}
