// Phase 0 spike: Teams-like chat screen in Slint, to judge Arabic/RTL text, IME input and RAM.
use slint::{Model, VecModel};
use std::rc::Rc;

mod mock {
    include!("../../shared/mock.rs");
}

slint::slint! {
    import { ListView, LineEdit, Button, Palette } from "std-widgets.slint";

    export struct Msg { sender: string, text: string, mine: bool, rtl: bool, time: string }
    export struct Chat { name: string, preview: string, unread: int, rtl: bool }

    export component AppWindow inherits Window {
        title: "kelma spike (Slint)";
        preferred-width: 1100px;
        preferred-height: 750px;
        background: #f5f5f5;
        in property <[Chat]> chats;
        in property <[Msg]> messages;
        in-out property <int> selected: 0;
        callback send(string);
        init => { Palette.color-scheme = ColorScheme.light; }

        HorizontalLayout {
            // Left rail
            Rectangle {
                width: 68px;
                background: #ebebeb;
                VerticalLayout {
                    padding-top: 12px;
                    spacing: 18px;
                    alignment: start;
                    Text { text: "💬\nChat"; horizontal-alignment: center; font-size: 12px; color: #5b5fc7; }
                    Text { text: "👥\nTeams"; horizontal-alignment: center; font-size: 12px; color: #424242; }
                }
            }
            // Chat list
            Rectangle {
                width: 300px;
                background: #fafafa;
                VerticalLayout {
                    Text { text: "Chat"; color: #242424; font-size: 18px; font-weight: 700; height: 48px; vertical-alignment: center; x: 16px; }
                    ListView {
                        for c[i] in chats: Rectangle {
                            height: 64px;
                            background: i == selected ? #ffffff : transparent;
                            TouchArea { clicked => { selected = i; } }
                            HorizontalLayout {
                                padding: 10px;
                                spacing: 10px;
                                Rectangle { width: 36px; height: 36px; border-radius: 18px; background: #c5cae9; }
                                VerticalLayout {
                                    Text { text: c.name; color: #242424; font-weight: 600; overflow: elide; horizontal-alignment: c.rtl ? right : left; }
                                    Text { text: c.preview; color: #616161; font-size: 12px; overflow: elide; }
                                }
                                if c.unread > 0: Rectangle {
                                    width: 22px; height: 18px; border-radius: 9px; background: #c4314b;
                                    Text { text: c.unread; color: white; font-size: 11px; }
                                }
                            }
                        }
                    }
                }
            }
            // Conversation
            VerticalLayout {
                Rectangle {
                    height: 48px;
                    background: #ffffff;
                    Text { text: chats[selected].name; color: #242424; font-size: 16px; font-weight: 700; x: 16px; }
                }
                ListView {
                    for m in messages: HorizontalLayout {
                        padding-left: 16px;
                        padding-right: 16px;
                        padding-top: 4px;
                        padding-bottom: 4px;
                        alignment: m.mine ? LayoutAlignment.end : LayoutAlignment.start;
                        Rectangle {
                            max-width: 520px;
                            border-radius: 6px;
                            background: m.mine ? #e8ebfa : #ffffff;
                            VerticalLayout {
                                padding: 10px;
                                spacing: 4px;
                                if !m.mine: Text { text: m.sender + "  " + m.time; font-size: 11px; color: #616161; }
                                Text {
                                    text: m.text;
                                    color: #242424;
                                    wrap: word-wrap;
                                    font-size: 14px;
                                    horizontal-alignment: m.rtl ? right : left;
                                }
                            }
                        }
                    }
                }
                HorizontalLayout {
                    padding: 12px;
                    spacing: 8px;
                    input := LineEdit {
                        placeholder-text: "Type a message / اكتب رسالة";
                        accepted(t) => { root.send(t); self.text = ""; }
                    }
                    Button {
                        text: "Send";
                        clicked => { root.send(input.text); input.text = ""; }
                    }
                }
            }
        }
    }
}

fn main() {
    let app = AppWindow::new().expect("create window");

    let chats: Vec<Chat> = mock::chats()
        .into_iter()
        .map(|c| Chat { name: c.name.into(), preview: c.preview.into(), unread: c.unread, rtl: mock::is_rtl(c.name) })
        .collect();
    app.set_chats(Rc::new(VecModel::from(chats)).into());

    let msgs: Vec<Msg> = mock::messages(300)
        .into_iter()
        .map(|m| Msg {
            sender: m.sender.into(),
            text: m.text.into(),
            mine: m.mine,
            rtl: mock::is_rtl(m.text),
            time: m.time.into(),
        })
        .collect();
    let model = Rc::new(VecModel::from(msgs));
    app.set_messages(model.clone().into());

    app.on_send(move |t| {
        if t.trim().is_empty() {
            return;
        }
        model.push(Msg { sender: "Me".into(), rtl: mock::is_rtl(&t), text: t, mine: true, time: "now".into() });
        let _ = model.row_count();
    });

    app.run().expect("run");
}
