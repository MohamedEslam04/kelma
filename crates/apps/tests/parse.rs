//! Parsers against synthetic fixtures shaped like the live answers (2026-11-19).

use apps::{
    AddTarget, AppDetails, Certification, DevicePermission, Distribution, EntitlementState, Filter,
    Grants, Price, Scope, catalog_apps, parse_app_policy, parse_definitions, parse_installed,
    parse_search, parse_store_home, parse_your_apps,
};
use serde_json::{Value, json};
use teams_proto::graph::values;

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn installed_apps_keep_entitlements_and_definitions() {
    let apps = parse_installed(&fixture("aggregated.json"));
    assert_eq!(apps.entitlements.len(), 10);
    assert_eq!(apps.definitions.len(), 9);
    assert_eq!(apps.hash.len(), 64);
    let planner = apps
        .entitlements
        .iter()
        .find(|e| e.id == "com.microsoft.teamspace.tab.planner")
        .unwrap();
    assert_eq!(planner.state, EntitlementState::Installed);
    assert!(planner.pinned && planner.user_pinned && !planner.admin_pinned);
    assert_eq!(planner.order, Some(4));
    assert_eq!(planner.installed_version.as_deref(), Some("1.8.0"));
    assert!(planner.has_update(apps.definition(&planner.id)));
}

#[test]
fn app_bar_is_the_pinned_apps_in_order() {
    let apps = parse_installed(&fixture("aggregated.json"));
    let bar: Vec<&str> = apps
        .app_bar()
        .iter()
        .map(|(_, d)| d.name.as_str())
        .collect();
    assert_eq!(bar, ["Activity", "Chat", "Planner"]);
    let (chat, _) = apps.app_bar()[1];
    assert!(chat.admin_pinned);
}

#[test]
fn more_apps_are_installed_personal_apps_not_pinned() {
    let apps = parse_installed(&fixture("aggregated.json"));
    let names = |q: &str| -> Vec<String> {
        apps.more_apps(q)
            .iter()
            .map(|d| d.name.clone())
            .collect::<Vec<_>>()
    };
    // Featured, pre-consented, deprecated, pinned, core and unknown apps stay out, and so do
    // apps without a personal tab or bot (a message extension only).
    assert_eq!(names(""), ["Contoso Tickets", "OneNote"]);
    assert_eq!(names("note"), ["OneNote"]);
    assert_eq!(names("NOTE"), ["OneNote"]);
    assert!(names("zzz").is_empty());
}

#[test]
fn definition_reads_the_app_page_fields() {
    let defs = parse_definitions(&fixture("definitions.json"));
    assert_eq!(defs.len(), 1);
    let d = &defs[0];
    assert_eq!(d.id, "0d820ecd-def2-4297-adad-78056cde7c78");
    assert_eq!(d.name, "OneNote");
    assert_eq!(d.developer, "Microsoft Corporation");
    assert_eq!(d.developer_url.as_deref(), Some("https://www.onenote.com/"));
    assert_eq!(
        d.privacy_url.as_deref(),
        Some("https://privacy.microsoft.com/privacystatement")
    );
    assert_eq!(
        d.terms_url.as_deref(),
        Some("https://www.microsoft.com/servicesagreement")
    );
    assert_eq!(
        d.long_description,
        "Microsoft OneNote is a digital notebook."
    );
    assert_eq!(d.accent_color, "#641790");
    assert!(d.large_icon.ends_with("color.png"));
    assert!(d.small_icon.ends_with("outline.png"));
    assert_eq!(d.screenshots.len(), 2);
    assert!(d.video_url.is_some());
    assert_eq!(d.categories, ["Productivity", "Microsoft"]);
    assert_eq!(d.permissions, ["Identity", "MessageTeamMembers"]);
    assert_eq!(
        d.rsc_permissions,
        ["ChannelMessage.Read.Group", "ChatMessage.Read.Chat"]
    );
    assert_eq!(
        d.device_permissions,
        [
            DevicePermission::Media,
            DevicePermission::Notifications,
            DevicePermission::Geolocation,
            DevicePermission::OpenExternal,
            DevicePermission::Midi,
        ]
    );
    assert!(d.is_microsoft && !d.is_core);
    assert!(d.works_in.personal && d.works_in.team && d.works_in.chat && d.works_in.meeting);
    assert!(d.has_tabs && !d.has_bot && !d.is_agent);
    assert_eq!(d.personal_tabs().count(), 1);
    assert_eq!(
        d.open_link("MyNotebook"),
        "https://teams.microsoft.com/l/entity/0d820ecd-def2-4297-adad-78056cde7c78/MyNotebook"
    );
    assert_eq!(
        d.share_link(),
        "https://teams.microsoft.com/l/app/0d820ecd-def2-4297-adad-78056cde7c78"
    );
    assert_eq!(d.raw["manifestVersion"], "devPreview");
}

#[test]
fn search_hits_carry_works_in_price_and_certification() {
    let hits = parse_search(&fixture("search.json"));
    assert_eq!(hits.len(), 2, "the hit without an id is dropped");
    let p = &hits[0];
    assert_eq!(p.id, "com.microsoft.teamspace.tab.planner");
    assert_eq!(p.name, "Planner");
    assert_eq!(p.developer, "Microsoft Corporation");
    assert_eq!(p.price, Price::Free);
    assert_eq!(p.certification, Certification::PublisherAttested);
    assert!(p.works_in.personal && p.works_in.team && p.works_in.meeting && !p.works_in.chat);
    assert!(!p.is_agent);
    assert_eq!(p.screenshots.len(), 1);
    let a = &hits[1];
    assert_eq!(a.price, Price::Paid);
    assert_eq!(a.certification, Certification::Certified);
    assert!(a.is_agent);
    assert!(a.works_in.personal && a.works_in.chat && !a.works_in.team);
    assert_eq!(
        a.title_id.as_deref(),
        Some("T_00000000-0000-0000-0000-000000000044")
    );
}

#[test]
fn store_home_lists_categories_with_ratings() {
    let home = parse_store_home(&fixture("store_home.json"));
    assert_eq!(home.categories.len(), 3);
    let all = home.category("all").unwrap();
    assert_eq!(all.total, 3149);
    assert_eq!(all.blocked, 28);
    assert_eq!(all.apps.len(), 2);
    let r = all.apps[0].rating.unwrap();
    assert_eq!(r.count, 119);
    assert!((r.average - 4.496).abs() < 1e-9);
    assert_eq!(all.apps[0].ranking, Some(3864));
    assert!(all.apps[1].is_blocked);
    assert!(all.apps[1].rating.is_none());
    assert_eq!(home.category("popularApps").unwrap().apps.len(), 1);
    assert!(home.category("nope").is_none());
}

#[test]
fn your_apps_lists_store_entitlements() {
    let mine = parse_your_apps(&fixture("your_apps.json"));
    assert_eq!(mine.len(), 2);
    assert_eq!(mine[0].id, "0d820ecd-def2-4297-adad-78056cde7c78");
    assert_eq!(mine[0].categories, ["Productivity"]);
    assert_eq!(mine[1].kinds, ["Bots"]);
}

#[test]
fn filter_by_works_in_and_agent() {
    let hits = parse_search(&fixture("search.json"));
    let keep = |f: Filter| -> Vec<&str> {
        hits.iter()
            .filter(|h| f.matches(h.works_in, h.is_agent))
            .map(|h| h.name.as_str())
            .collect()
    };
    assert_eq!(keep(Filter::default()).len(), 2);
    assert_eq!(
        keep(Filter {
            scope: Some(Scope::Team),
            agents: None
        }),
        ["Planner"]
    );
    assert_eq!(
        keep(Filter {
            scope: Some(Scope::Chat),
            agents: None
        }),
        ["Contoso Planner Agent"]
    );
    assert_eq!(
        keep(Filter {
            scope: None,
            agents: Some(true)
        }),
        ["Contoso Planner Agent"]
    );
    assert_eq!(
        keep(Filter {
            scope: Some(Scope::Personal),
            agents: Some(false)
        }),
        ["Planner"]
    );
}

#[test]
fn details_merge_definition_store_entry_and_search_hit() {
    let def = parse_definitions(&fixture("definitions.json")).remove(0);
    let details = AppDetails::new(def, None, None);
    assert_eq!(details.price, Price::Unknown);
    assert!(details.rating.is_none());
    assert_eq!(
        details.add_targets(),
        [
            AddTarget::Me,
            AddTarget::Team,
            AddTarget::Chat,
            AddTarget::Meeting
        ]
    );

    let home = parse_store_home(&fixture("store_home.json"));
    let hits = parse_search(&fixture("search.json"));
    let planner = json!({"id": "com.microsoft.teamspace.tab.planner", "name": "Planner",
        "metaCapabilities": {"staticTabs": {"scopes": ["Personal"]}}});
    let def = apps::AppDefinition::parse(&planner).unwrap();
    let entry = home.category("all").unwrap().apps.first();
    let details = AppDetails::new(def, entry, hits.first());
    assert_eq!(details.rating.unwrap().count, 119);
    assert_eq!(details.price, Price::Free);
    assert_eq!(details.certification, Certification::PublisherAttested);
    assert_eq!(details.add_targets(), [AddTarget::Me]);
}

#[test]
fn graph_catalog_keeps_apps_with_an_id() {
    let apps = catalog_apps(values(&fixture("graph_catalog.json")));
    assert_eq!(apps.len(), 3, "the entry without an id is dropped");
    let planner = &apps[0];
    assert_eq!(planner.id, "com.microsoft.teamspace.tab.planner");
    assert_eq!(planner.name, "Planner");
    assert_eq!(planner.distribution, Distribution::Store);
    assert_eq!(planner.version, "1.0");
    assert_eq!(planner.short_description, "Plan and track work");
    assert!(planner.has_bot);
    assert_eq!(planner.rsc_permissions, ["ChannelMessage.Read.Group"]);
    let org = &apps[1];
    assert_eq!(org.distribution, Distribution::Organization);
    assert_eq!(
        org.external_id.as_deref(),
        Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
    );
    let bare = &apps[2];
    assert_eq!(bare.distribution, Distribution::Sideloaded);
    assert_eq!(bare.version, "");
}

#[test]
fn app_policy_and_local_grants() {
    let p = parse_app_policy(&fixture("app_policy.json"));
    assert!(!p.sideloading_allowed);
    assert!(p.user_pinning_allowed);
    assert!(parse_app_policy(&json!({})).user_pinning_allowed);

    let mut g = Grants::default();
    assert!(
        !g.allowed("app-1", DevicePermission::Media),
        "denied by default"
    );
    g.set("app-1", DevicePermission::Media, true);
    g.set("app-1", DevicePermission::Geolocation, false);
    assert!(g.allowed("app-1", DevicePermission::Media));
    let back = Grants::from_json(&g.to_json());
    assert_eq!(back, g);
    assert_eq!(
        back.for_app(
            "app-1",
            &[DevicePermission::Media, DevicePermission::Notifications]
        ),
        [
            (DevicePermission::Media, true),
            (DevicePermission::Notifications, false)
        ]
    );
    assert_eq!(Grants::from_json(&json!("garbage")), Grants::default());
}
