use zintl_native::{Context, MessageHandler, PlatformMessageLoop};

enum Message {}

struct Handler {}

impl MessageHandler<Message> for Handler {
    fn on_init(&mut self, cx: impl Context<Message>) {
        let wm = cx.window_manager();
        cx.perform_main(
            move |marker| {
                let window = wm.create_window(marker);
                window.read(marker).unwrap().show();
            },
            None,
        );
    }
}

fn main() {
    let handler = Handler {};
    let m = PlatformMessageLoop::new(handler);
    m.run();
}
