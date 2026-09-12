use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, ThreadId};

use zintl_ui_desktop::*;

pub struct MainView {
    main_thread: ThreadId,
    performed: Arc<AtomicBool>,
    status: Store<String>,
}

impl MainView {
    pub fn new() -> Self {
        Self {
            main_thread: thread::current().id(),
            performed: Arc::new(AtomicBool::new(false)),
            status: Store::default(),
        }
    }
}

impl View for MainView {
    type Output = RenderNode;

    fn init(&mut self, cx: &mut Context<'_>) {
        self.status = cx.store("Pending".to_owned());
    }

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
        let main_thread = self.main_thread;
        let performed = self.performed.clone();
        let observed = self.performed.clone();
        let status = self.status;

        Window::new(
            Rect::new(100.0, 100.0, 480.0, 240.0),
            "Zintl Perform Main UI Test",
        )
        .id("main-window")
        .content(
            VStack::new(list![
                Button::new("Schedule main task")
                    .id("schedule-main-task")
                    .on_click(move |cx| {
                        let performed = performed.clone();
                        cx.perform_main(move || {
                            assert_eq!(thread::current().id(), main_thread);
                            performed.store(true, Ordering::SeqCst);
                        });
                    }),
                Button::new("Observe result")
                    .id("observe-main-task")
                    .on_click(move |cx| {
                        let value = if observed.load(Ordering::SeqCst) {
                            "Performed"
                        } else {
                            "Pending"
                        };
                        cx.update(status, |status| *status = value.to_owned());
                    }),
                cx.watch(status, |status| {
                    Text::new(status.clone()).id("main-task-status")
                }),
            ])
            .spacing(16.0),
        )
    }
}
