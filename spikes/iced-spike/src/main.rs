// Phase 0 spike: Teams-like chat screen in iced, to judge Arabic/RTL text, IME input and RAM.
use iced::widget::{button, column, container, row, scrollable, text, text_input, Column};
use iced::{alignment, Background, Border, Color, Element, Length, Theme};

mod mock {
    include!("../../shared/mock.rs");
}

struct Msg {
    sender: String,
    text: String,
    mine: bool,
    rtl: bool,
    time: String,
}

struct App {
    chats: Vec<mock::MockChat>,
    messages: Vec<Msg>,
    selected: usize,
    draft: String,
}

#[derive(Debug, Clone)]
enum Message {
    Select(usize),
    Draft(String),
    Send,
}

impl Default for App {
    fn default() -> Self {
        let messages = mock::messages(300)
            .into_iter()
            .map(|m| Msg { sender: m.sender.into(), rtl: mock::is_rtl(m.text), text: m.text.into(), mine: m.mine, time: m.time })
            .collect();
        App { chats: mock::chats(), messages, selected: 0, draft: String::new() }
    }
}

fn rgb(hex: u32) -> Color {
    Color::from_rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn bg(hex: u32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(Background::Color(rgb(hex))), ..Default::default() }
}

fn update(app: &mut App, msg: Message) {
    match msg {
        Message::Select(i) => app.selected = i,
        Message::Draft(s) => app.draft = s,
        Message::Send => {
            let t = std::mem::take(&mut app.draft);
            if !t.trim().is_empty() {
                app.messages.push(Msg { sender: "Me".into(), rtl: mock::is_rtl(&t), text: t, mine: true, time: "now".into() });
            }
        }
    }
}

fn view(app: &App) -> Element<'_, Message> {
    let rail = container(
        column![text("💬\nChat").size(12).color(rgb(0x5b5fc7)), text("👥\nTeams").size(12)]
            .spacing(18)
            .align_x(alignment::Horizontal::Center),
    )
    .padding(12)
    .width(68)
    .height(Length::Fill)
    .style(bg(0xebebeb));

    let chat_items = app.chats.iter().enumerate().map(|(i, c)| {
        let mut r = row![
            container(text("")).width(36).height(36).style(|_: &Theme| container::Style {
                background: Some(Background::Color(rgb(0xc5cae9))),
                border: Border { radius: 18.0.into(), ..Default::default() },
                ..Default::default()
            }),
            column![text(c.name).size(14), text(c.preview).size(12).color(rgb(0x616161))].width(Length::Fill),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center);
        if c.unread > 0 {
            r = r.push(container(text(c.unread.to_string()).size(11).color(Color::WHITE)).padding([1, 6]).style(
                |_: &Theme| container::Style {
                    background: Some(Background::Color(rgb(0xc4314b))),
                    border: Border { radius: 9.0.into(), ..Default::default() },
                    ..Default::default()
                },
            ));
        }
        button(r)
            .on_press(Message::Select(i))
            .padding(10)
            .width(Length::Fill)
            .style(move |theme, status| {
                let mut s = button::text(theme, status);
                if i == app.selected {
                    s.background = Some(Background::Color(Color::WHITE));
                }
                s
            })
            .into()
    });
    let chat_list = container(
        column![
            container(text("Chat").size(18)).padding(12),
            scrollable(Column::with_children(chat_items)).height(Length::Fill)
        ],
    )
    .width(300)
    .height(Length::Fill)
    .style(bg(0xfafafa));

    let bubbles = app.messages.iter().map(|m| {
        let align = if m.rtl { alignment::Horizontal::Right } else { alignment::Horizontal::Left };
        let mut col = Column::new().spacing(4);
        if !m.mine {
            col = col.push(text(format!("{}  {}", m.sender, m.time)).size(11).color(rgb(0x616161)));
        }
        col = col.push(text(&m.text).size(14).align_x(align));
        let bubble = container(col).padding(10).max_width(520).style(move |_: &Theme| container::Style {
            background: Some(Background::Color(if m.mine { rgb(0xe8ebfa) } else { Color::WHITE })),
            border: Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        });
        container(bubble)
            .width(Length::Fill)
            .padding([4, 16])
            .align_x(if m.mine { alignment::Horizontal::Right } else { alignment::Horizontal::Left })
            .into()
    });

    let header = container(text(app.chats[app.selected].name).size(16)).padding(14).width(Length::Fill).style(bg(0xffffff));
    let composer = row![
        text_input("Type a message / اكتب رسالة", &app.draft).on_input(Message::Draft).on_submit(Message::Send).padding(8),
        button("Send").on_press(Message::Send),
    ]
    .spacing(8)
    .padding(12);

    let conversation = column![
        header,
        scrollable(Column::with_children(bubbles)).height(Length::Fill).anchor_bottom(),
        composer
    ];

    container(row![rail, chat_list, conversation]).style(bg(0xf5f5f5)).into()
}

fn main() -> iced::Result {
    iced::application(App::default, update, view).title("kelma spike (iced)").theme(|_: &App| Theme::Light).window_size((1100.0, 750.0)).run()
}
