// Mock chat data shared by the UI toolkit spikes (included with `include!`).
// Mixes Arabic, English, mixed-direction text, emoji, numbers and long paragraphs.

pub struct MockMsg {
    pub sender: &'static str,
    pub text: &'static str,
    pub mine: bool,
    pub time: String,
}

pub struct MockChat {
    pub name: &'static str,
    pub preview: &'static str,
    pub unread: i32,
}

const LINES: &[(&str, &str, bool)] = &[
    ("Ahmed", "صباح الخير يا جماعة، الـ deploy خلص؟", false),
    ("Sara", "Good morning! Yes, it was deployed to staging at 10:30.", false),
    ("Me", "تمام 👍 هراجع الـ PR بتاع CPS-122 النهارده", true),
    ("Omar", "Can you check the Redis memory graph? It spiked to 80% after the release.", false),
    ("Me", "الـ API بترجع 500 لما الـ token يخلص، محتاجين refresh logic قبل ما نعمل release.", true),
    ("Ahmed", "مين فاضي يعمل review؟ 🙏", false),
    ("Sara", "Meeting moved to 3 PM — the link is in the calendar invite.", false),
    ("Me", "شكراً جداً ❤️", true),
    (
        "Omar",
        "بالنسبة للـ architecture اللي اتكلمنا عليها امبارح، أنا شايف إن الأحسن نفصل الـ worker عن الـ API server \
         عشان لو الـ queue اتملت ما تأثرش على الـ response time، وكمان نقدر نعمل scale لكل واحد لوحده \
         حسب الـ load. ده هيحتاج نعدل الـ Helm chart ونضيف HPA للـ worker.",
        false,
    ),
    ("Me", "RAM وصلت 74% والـ swap 7.5G — هقفل Teams وأشوف 😅", true),
    ("Sara", "LGTM, merging now. 🚀", false),
    ("Ahmed", "تمام، I'll update the ticket and ping QA.", false),
    ("Me", "Quick question: هل الـ endpoint ده محتاج auth ولا public؟", true),
    (
        "Sara",
        "Here is a longer English paragraph to check wrapping: the migration runs in three steps, first we \
         backfill the new column, then we switch reads, and finally we drop the old column after a week.",
        false,
    ),
];

pub fn messages(n: usize) -> Vec<MockMsg> {
    (0..n)
        .map(|i| {
            let (sender, text, mine) = LINES[i % LINES.len()];
            MockMsg { sender, text, mine, time: format!("{:02}:{:02}", 9 + (i / 60) % 12, i % 60) }
        })
        .collect()
}

pub fn chats() -> Vec<MockChat> {
    vec![
        MockChat { name: "Backend Team", preview: "مين فاضي يعمل review؟ 🙏", unread: 3 },
        MockChat { name: "Ahmed Ali", preview: "تمام، I'll update the ticket", unread: 0 },
        MockChat { name: "Sara Hassan", preview: "LGTM, merging now. 🚀", unread: 1 },
        MockChat { name: "Omar Khaled", preview: "بالنسبة للـ architecture…", unread: 0 },
        MockChat { name: "Release Squad", preview: "Meeting moved to 3 PM", unread: 12 },
        MockChat { name: "QA", preview: "Bug reproduced on staging", unread: 0 },
        MockChat { name: "مجموعة الإدارة", preview: "اجتماع بكرة الساعة ١٠", unread: 2 },
        MockChat { name: "Notes", preview: "self notes", unread: 0 },
    ]
}

/// Base direction of a paragraph from its first strong character (UAX #9 rule P2).
pub fn is_rtl(text: &str) -> bool {
    for c in text.chars() {
        let u = c as u32;
        let rtl = (0x0590..=0x08FF).contains(&u) || (0xFB1D..=0xFDFF).contains(&u) || (0xFE70..=0xFEFF).contains(&u);
        if rtl {
            return true;
        }
        if c.is_alphabetic() {
            return false;
        }
    }
    false
}
