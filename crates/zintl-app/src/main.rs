use zintl_native::{Context, Event, MainActor, MessageHandler, PlatformMessageLoop, Window};

enum Message {
    CreateWindow(MainActor<Window>),
}

#[derive(Default)]
struct Handler {
    window: Option<MainActor<Window>>,
}

impl MessageHandler<Message> for Handler {
    fn on_init(&mut self, cx: impl Context<Message>) {
        let wm = cx.window_manager();
        cx.perform_main(
            move |marker, cx| {
                let window = wm.create_window(marker);
                window.read(marker).unwrap().show();
                cx.send_message(Message::CreateWindow(window));
            },
            None,
        );
    }

    fn on_event(&mut self, _cx: impl Context<Message>, event: Event<Message>) {
        match event {
            Event::UserMessage(Message::CreateWindow(window)) => {
                self.window = Some(window);
            }
        }
    }
}

fn main() {
    let handler = Handler::default();
    let m = PlatformMessageLoop::new(handler);
    m.run();
}
