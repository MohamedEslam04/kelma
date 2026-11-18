//! UI-thread side: turns view data into Slint models.

use crate::data::{ChatData, HeaderData, MsgData, Pixels, TeamData};
use crate::{AppWindow, ChannelRow, ChatRow, FileItem, ImageItem, Msg, ReactionItem, TeamRow};
use slint::{
    Color, ComponentHandle, Image, Model, ModelRc, SharedString, StyledText, VecModel, Weak,
};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static MESSAGES: RefCell<Option<Rc<VecModel<Msg>>>> = const { RefCell::new(None) };
}

fn rgb(c: u32) -> Color {
    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

fn img(p: &Option<Pixels>) -> (Image, bool) {
    match p {
        Some(px) => (Image::from_rgba8(px.clone()), true),
        None => (Image::default(), false),
    }
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn chat_row(c: ChatData) -> ChatRow {
    let (avatar, has_avatar) = img(&c.avatar);
    ChatRow {
        id: c.id.into(),
        title: c.title.into(),
        preview: c.preview.into(),
        time: c.time.into(),
        unread: c.unread,
        initials: c.initials.into(),
        color: rgb(c.color),
        avatar,
        has_avatar,
        section: c.section.into(),
        rtl_title: c.rtl_title,
        rtl_preview: c.rtl_preview,
        favorite: c.favorite,
        draft: c.draft,
    }
}

fn styled(markdown: &str) -> StyledText {
    StyledText::from_markdown(markdown).unwrap_or_else(|_| StyledText::from_plain_text(markdown))
}

fn msg(m: MsgData) -> Msg {
    let (avatar, has_avatar) = img(&m.avatar);
    Msg {
        id: m.id.into(),
        sender: m.sender.into(),
        initials: m.initials.into(),
        color: rgb(m.color),
        avatar,
        has_avatar,
        mine: m.mine,
        time: m.time.into(),
        day: m.day.into(),
        show_header: m.show_header,
        edited: m.edited,
        deleted: m.deleted,
        system: m.system,
        subject: m.subject.into(),
        quote_author: m.quote_author.into(),
        quote_text: m.quote_text.into(),
        body: styled(&m.markdown),
        plain: m.plain.into(),
        rtl: m.rtl,
        images: model(
            m.images
                .into_iter()
                .map(|i| {
                    let (image, loaded) = img(&i.pixels);
                    ImageItem {
                        url: i.url.into(),
                        img: image,
                        loaded,
                        w: i.w,
                        h: i.h,
                    }
                })
                .collect(),
        ),
        files: model(
            m.files
                .into_iter()
                .map(|f| FileItem {
                    name: f.name.into(),
                    url: f.url.into(),
                    ext: f.ext.into(),
                })
                .collect(),
        ),
        reactions: model(
            m.reactions
                .into_iter()
                .map(|r| ReactionItem {
                    key: r.key.into(),
                    emoji: r.emoji.into(),
                    count: r.count,
                    mine: r.mine,
                })
                .collect(),
        ),
        reply_count: m.reply_count,
        pending: m.pending,
    }
}

/// Thread-safe handle the backend uses to update the window.
#[derive(Clone)]
pub struct UiHandle {
    weak: Weak<AppWindow>,
}

impl UiHandle {
    pub fn new(weak: Weak<AppWindow>) -> Self {
        UiHandle { weak }
    }

    fn run(&self, f: impl FnOnce(AppWindow) + Send + 'static) {
        // Tests drive the window on their own thread without an event loop.
        #[cfg(test)]
        if let Some(w) = self.weak.upgrade() {
            return f(w);
        }
        let _ = self.weak.upgrade_in_event_loop(f);
    }

    pub fn login_checking(&self) {
        self.run(|w| w.set_login_state("checking".into()));
    }

    pub fn login_code(&self, code: String, url: String) {
        self.run(move |w| {
            w.set_user_code(code.into());
            w.set_verify_url(url.into());
            w.set_login_state("code".into());
        });
    }

    pub fn login_error(&self, err: String) {
        self.run(move |w| {
            w.set_login_error(err.into());
            w.set_login_state("error".into());
        });
    }

    pub fn ready(&self, my_initials: String) {
        self.run(move |w| {
            w.set_me_initials(my_initials.into());
            w.set_login_state("ready".into());
        });
    }

    /// Shows a transient message for 8 seconds.
    pub fn banner(&self, text: String) {
        self.run(move |w| {
            let shown: SharedString = text.into();
            w.set_banner(shown.clone());
            let weak = w.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_secs(8), move || {
                if let Some(w) = weak.upgrade()
                    && w.get_banner() == shown
                {
                    w.set_banner(SharedString::new());
                }
            });
        });
    }

    pub fn connected(&self, ok: bool) {
        self.run(move |w| w.set_connected(ok));
    }

    pub fn typing(&self, text: String) {
        self.run(move |w| w.set_typing(text.into()));
    }

    pub fn loading_older(&self, on: bool) {
        self.run(move |w| w.set_loading_older(on));
    }

    pub fn focus(&self) {
        self.run(|w| w.set_focus_tick(w.get_focus_tick() + 1));
    }

    pub fn set_draft(&self, text: String) {
        self.run(move |w| w.set_draft(text.into()));
    }

    pub fn enter_sends(&self, on: bool) {
        self.run(move |w| w.set_enter_sends(on));
    }

    pub fn clear_subject(&self) {
        self.run(|w| w.set_post_subject(SharedString::new()));
    }

    pub fn reply_bar(&self, author: String, preview: String, editing: bool) {
        self.run(move |w| {
            w.set_reply_author(author.into());
            w.set_reply_preview(preview.into());
            if !editing {
                w.set_editing_id(SharedString::new());
            } else {
                w.set_editing_id("1".into());
            }
        });
    }

    pub fn chats(&self, fav: Vec<ChatData>, main: Vec<ChatData>, chans: Vec<ChatData>) {
        self.run(move |w| {
            w.set_chats_favorites(model(fav.into_iter().map(chat_row).collect()));
            w.set_chats_main(model(main.into_iter().map(chat_row).collect()));
            w.set_chats_channels(model(chans.into_iter().map(chat_row).collect()));
        });
    }

    pub fn teams(&self, teams: Vec<TeamData>) {
        self.run(move |w| {
            w.set_teams(model(
                teams
                    .into_iter()
                    .map(|t| TeamRow {
                        id: t.id.into(),
                        name: t.name.into(),
                        initials: t.initials.into(),
                        color: rgb(t.color),
                        expanded: t.expanded,
                        channels: model(
                            t.channels
                                .into_iter()
                                .map(|(id, name, unread)| ChannelRow {
                                    id: id.into(),
                                    name: name.into(),
                                    unread,
                                })
                                .collect(),
                        ),
                    })
                    .collect(),
            ));
        });
    }

    pub fn header(&self, h: HeaderData) {
        self.run(move |w| {
            let (avatar, has) = img(&h.avatar);
            w.set_current_id(h.id.into());
            w.set_current_title(h.title.into());
            w.set_current_initials(h.initials.into());
            w.set_current_color(rgb(h.color));
            w.set_current_avatar(avatar);
            w.set_current_has_avatar(has);
            w.set_current_is_channel(h.is_channel);
            w.set_thread_root(h.thread_root.into());
        });
    }

    /// Replaces the message list, updating rows in place when the ids are unchanged so the
    /// scroll position is kept.
    pub fn messages(&self, msgs: Vec<MsgData>, scroll: bool) {
        self.run(move |w| {
            MESSAGES.with(|cell| {
                let mut cell = cell.borrow_mut();
                let m = cell.get_or_insert_with(|| {
                    let m = Rc::new(VecModel::default());
                    w.set_messages(ModelRc::from(m.clone()));
                    m
                });
                let same_ids = m.row_count() == msgs.len()
                    && msgs
                        .iter()
                        .enumerate()
                        .all(|(i, x)| m.row_data(i).is_some_and(|r| r.id == x.id.as_str()));
                if same_ids {
                    for (i, x) in msgs.into_iter().enumerate() {
                        m.set_row_data(i, msg(x));
                    }
                } else {
                    m.set_vec(msgs.into_iter().map(msg).collect::<Vec<_>>());
                }
            });
            if scroll {
                // Two ticks: one now, one after layout settles with images.
                w.set_scroll_tick(w.get_scroll_tick() + 1);
                let weak = w.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_millis(150), move || {
                    if let Some(w) = weak.upgrade() {
                        w.set_scroll_tick(w.get_scroll_tick() + 1);
                    }
                });
            }
        });
    }
}
