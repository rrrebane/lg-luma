use super::*;
use serde_json::json;

fn entry(id: &str, ts: i64) -> Value {
    json!({
        "id": id, "title": format!("Video {id}"), "type": "episode", "domain": "jupiter.err.ee",
        "url": format!("https://jupiter.err.ee/{id}"), "img": format!("https://s.err.ee/photo/{id}.jpg"),
        "t": 120.0, "d": 600.0, "ts": ts,
    })
}

fn resume_in(name: &str) -> Resume {
    let dir = std::env::temp_dir().join(format!("luma-resume-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    Resume::new(&dir.join("preferences.json"))
}

#[test]
fn an_app_list_is_validated_sorted_and_bounded() {
    let mut list: Vec<Value> = (0..40).map(|n| entry(&n.to_string(), n)).collect();
    list.push(json!({"id": "bad id!", "title": "x", "type": "movie", "domain": "jupiter.err.ee"}));
    list.push(json!({"id": "1", "title": "x", "type": "game", "domain": "jupiter.err.ee"}));
    list.push(json!({"id": "2", "title": "x", "type": "movie", "domain": "evil.example.com"}));
    list.push(json!({"id": "3", "title": "x\u{7}", "type": "movie", "domain": "jupiter.err.ee"}));
    let items = parse_items(Source::Jupiter, &Value::Array(list));
    assert_eq!(items.len(), MAX_ITEMS_PER_APP);
    assert_eq!(items[0].id, "39", "newest first");
    assert!(items.iter().all(|item| item.id.parse::<u32>().is_ok()));
    assert!(parse_items(Source::Jupiter, &json!("nope")).is_empty());
}

#[test]
fn addresses_must_be_https_on_an_err_host() {
    let mut value = entry("1", 1);
    value["img"] = json!("https://evil.example.com/a.jpg");
    value["url"] = json!("http://jupiter.err.ee/1");
    let item = parse_item(Source::Jupiter, &value).unwrap();
    assert_eq!(item.image, "");
    assert_eq!(item.url, "");
    for bad in [
        "https://user@s.err.ee/a.jpg",
        "https://s.err.ee:8443/a.jpg",
        "https://err.ee/a.jpg",
        "https://s.err.ee.evil.com/a.jpg",
        "https://s.err.ee/a b.jpg",
    ] {
        assert_eq!(err_url(bad), None, "{bad}");
    }
    assert!(err_url("https://s.err.ee/photo/crop/a.jpg").is_some());
}

#[test]
fn progress_is_a_fraction_and_recommendations_have_none() {
    let item = parse_item(Source::Jupiter, &entry("1", 1)).unwrap();
    assert!((item.progress() - 0.2).abs() < 1e-6);
    let mut resume = resume_in("progress");
    resume.accept_items(Source::Jupiter, vec![item]);
    let cards = resume.watch_cards();
    assert_eq!(cards[0].progress.map(|p| (p * 10.).round()), Some(2.));
    assert_eq!(cards[0].caption, "ERR Jupiter");
    assert_eq!(cards[0].params["open"]["id"], "1");
    assert_eq!(cards[0].params["open"]["type"], "episode");
}

#[test]
fn watch_row_mixes_both_apps_newest_first_and_stops_at_five() {
    let mut resume = resume_in("watch");
    let jupiter: Vec<_> = (0..4)
        .map(|n| parse_item(Source::Jupiter, &entry(&format!("j{n}"), 100 + n * 10)).unwrap())
        .collect();
    let mut kids = entry("k1", 125);
    kids["domain"] = json!("lasteekraan.err.ee");
    let mut kids2 = entry("k2", 500);
    kids2["domain"] = json!("lasteekraan.err.ee");
    resume.accept_items(Source::Jupiter, jupiter);
    resume.accept_items(
        Source::Lasteekraan,
        vec![
            parse_item(Source::Lasteekraan, &kids).unwrap(),
            parse_item(Source::Lasteekraan, &kids2).unwrap(),
        ],
    );
    let order: Vec<_> = resume
        .watch_cards()
        .iter()
        .map(|card| card.key.clone())
        .collect();
    assert_eq!(
        order,
        [
            "watch:lasteekraan:k2",
            "watch:jupiter:j3",
            "watch:lasteekraan:k1",
            "watch:jupiter:j2",
            "watch:jupiter:j1"
        ]
    );
}

#[test]
fn kids_row_has_unfinished_lasteekraan_videos_then_recommendations() {
    let mut resume = resume_in("kids");
    let mut started = entry("k1", 5);
    started["domain"] = json!("lasteekraan.err.ee");
    resume.accept_items(
        Source::Lasteekraan,
        vec![parse_item(Source::Lasteekraan, &started).unwrap()],
    );
    resume.accept_items(
        Source::Jupiter,
        vec![parse_item(Source::Jupiter, &entry("j1", 9)).unwrap()],
    );
    let picks = json!({"data": {"category": {"frontPage": [
        {"header": "Midagi muud", "data": [{"id": "x", "type": "movie", "heading": "Ei"}]},
        {"header": PICKS_SHELF, "data": [
            {"id": "k1", "type": "series", "heading": "Juba pooleli"},
            {"id": 77, "type": "movie", "heading": "Uus film", "photos": [{"photoTypes": {
                "a": {"w": 600, "h": 338, "url": "https://s.err.ee/photo/77-600.jpg"},
                "b": {"w": 1920, "h": 1080, "url": "https://s.err.ee/photo/77-1920.jpg"},
                "c": {"w": 300, "h": 169, "url": "https://s.err.ee/photo/77-300.jpg"}}}]},
            {"id": "art", "type": "content", "heading": "Artikkel"}
        ]}
    ]}}});
    resume.accept_picks(parse_picks(&picks), 0);
    let cards = resume.kids_cards();
    let keys: Vec<_> = cards.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, ["kids:lasteekraan:k1", "kids:lasteekraan:77"]);
    assert!(cards[0].progress.is_some());
    assert_eq!(cards[1].progress, None);
    let pick = resume.find("kids:lasteekraan:77").unwrap();
    assert_eq!(
        pick.params["open"]["img"],
        "https://s.err.ee/photo/77-600.jpg"
    );
    assert_eq!(pick.params["open"]["type"], "movie");
    assert!(resume.find("kids:jupiter:j1").is_none());
}

#[test]
fn picks_need_the_recommendations_row() {
    assert!(parse_picks(&json!({"data": {"category": {"frontPage": []}}})).is_err());
    assert!(parse_picks(&json!({})).is_err());
}

#[test]
fn the_list_survives_a_restart() {
    let mut resume = resume_in("restart");
    resume.accept_items(
        Source::Jupiter,
        vec![parse_item(Source::Jupiter, &entry("5", 7)).unwrap()],
    );
    resume.save().unwrap();
    let again = Resume::new(&resume.file);
    assert_eq!(again.watch_cards()[0].key, "watch:jupiter:5");
    assert!(!resume.accept_items(
        Source::Jupiter,
        vec![parse_item(Source::Jupiter, &entry("5", 7)).unwrap()]
    ));
}

#[test]
fn downloads_wait_for_home_and_are_not_repeated() {
    let mut resume = resume_in("sync");
    resume.accept_items(
        Source::Jupiter,
        vec![parse_item(Source::Jupiter, &entry("1", 1)).unwrap()],
    );
    assert!(resume.sync(false, 0).is_empty());
    let first = resume.sync(true, 0);
    assert_eq!(first.len(), 2, "recommendations and one image");
    assert!(resume.sync(true, 1).is_empty(), "already asked for");
    resume.accept_image("https://s.err.ee/photo/1.jpg", false, 10);
    assert!(resume.sync(true, 11).is_empty(), "failed ones wait");
    assert_eq!(resume.sync(true, 10 + IMAGE_RETRY).len(), 1);
}

#[test]
fn numeric_ids_are_accepted_as_the_apps_store_them() {
    let mut value = entry("1", 1);
    value["id"] = json!(1609570862_u64);
    let item = parse_item(Source::Jupiter, &value).unwrap();
    assert_eq!(item.id, "1609570862");
    value["id"] = json!(-5);
    assert!(parse_item(Source::Jupiter, &value).is_none());
    value["id"] = json!(1.5);
    assert!(parse_item(Source::Jupiter, &value).is_none());
}

const TOKEN: &str = "abcdEFGH1234-_xyz";

fn plex() -> Plex {
    plex_from(Some("http://192.168.1.20:32400/"), Some(TOKEN)).unwrap()
}

#[test]
fn the_plex_server_must_be_on_the_home_network_unless_it_is_https() {
    assert_eq!(plex().base, "http://192.168.1.20:32400");
    for ok in [
        "http://10.0.0.5:32400",
        "http://plexbox:32400",
        "http://nas.local:32400",
        "https://plex.example.com",
        "https://plex.example.com:32400",
    ] {
        assert!(plex_from(Some(ok), Some(TOKEN)).is_some(), "{ok}");
    }
    for bad in [
        "http://plex.example.com:32400",
        "http://8.8.8.8:32400",
        "http://user@192.168.1.20:32400",
        "http://192.168.1.20:32400/library",
        "ftp://192.168.1.20",
        "192.168.1.20:32400",
        "http://192.168.1.20:abc",
    ] {
        assert!(plex_from(Some(bad), Some(TOKEN)).is_none(), "{bad}");
    }
    assert!(plex_from(Some("http://192.168.1.20:32400"), None).is_none());
    assert!(plex_from(Some("http://192.168.1.20:32400"), Some("short")).is_none());
    assert!(
        plex_from(
            Some("http://192.168.1.20:32400"),
            Some("has space in token!")
        )
        .is_none()
    );
    assert!(
        !format!("{:?}", plex()).contains(TOKEN),
        "the token must not reach a log"
    );
}

fn on_deck() -> Value {
    json!({"MediaContainer": {"Metadata": [
        {"type": "episode", "ratingKey": "812", "title": "Pilot", "grandparentTitle": "Severance",
         "parentIndex": 1, "index": 2, "viewOffset": 600000, "duration": 3000000,
         "lastViewedAt": 1700000500, "thumb": "/library/metadata/812/thumb/1"},
        {"type": "movie", "ratingKey": 77, "title": "Heat", "art": "/library/metadata/77/art/9 x",
         "duration": 7200000, "lastViewedAt": 1700000900},
        {"type": "track", "ratingKey": "1", "title": "A song"},
        {"type": "movie", "ratingKey": "bad id", "title": "Nope"}
    ]}})
}

#[test]
fn on_deck_gives_episodes_and_movies_with_artwork_addresses() {
    let items = parse_on_deck(&on_deck(), "http://192.168.1.20:32400").unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].title, "Severance S1E2");
    assert_eq!(items[0].position, 600.);
    assert_eq!(items[0].duration, 3000.);
    assert_eq!(items[0].updated, 1_700_000_500_000);
    assert!(
        items[0]
            .image
            .ends_with("&url=%2Flibrary%2Fmetadata%2F812%2Fthumb%2F1")
    );
    assert!(
        items[0]
            .image
            .starts_with("http://192.168.1.20:32400/photo/:/transcode?")
    );
    assert_eq!(items[1].id, "77");
    assert!(
        items[1].image.ends_with("art%2F9%20x"),
        "{}",
        items[1].image
    );
    assert!(!items[0].image.contains(TOKEN));
    assert!(parse_on_deck(&json!({}), "http://x").is_err());
}

#[test]
fn plex_cards_join_the_watch_row_and_only_open_the_app() {
    let mut resume = resume_in("plex");
    resume.accept_items(
        Source::Jupiter,
        vec![parse_item(Source::Jupiter, &entry("j1", 1_700_000_600_000)).unwrap()],
    );
    assert!(resume.set_plex(Some(plex())));
    assert!(
        !resume.set_plex(Some(plex())),
        "unchanged settings change nothing"
    );
    let items = parse_on_deck(&on_deck(), "http://192.168.1.20:32400").unwrap();
    assert!(resume.accept_plex(Ok(items.clone()), 0));
    assert!(
        !resume.accept_plex(Ok(items), 1),
        "same list, nothing to redraw"
    );
    let cards = resume.watch_cards();
    let keys: Vec<_> = cards.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(
        keys,
        ["watch:plex:77", "watch:jupiter:j1", "watch:plex:812"]
    );
    assert_eq!(cards[0].caption, "Plex");
    assert_eq!(cards[0].params, json!({}));
    assert_eq!(cards[0].source.app_id(), "com.beb.plxnative");
    assert!(
        resume.kids_cards().is_empty(),
        "Plex never reaches the kids row"
    );
    assert!(Source::parse("plex").is_none(), "apps cannot post as Plex");
}

#[test]
fn plex_is_fetched_with_its_token_only_for_its_own_artwork() {
    let mut resume = resume_in("plex-sync");
    resume.set_plex(Some(plex()));
    resume.accept_items(
        Source::Jupiter,
        vec![parse_item(Source::Jupiter, &entry("1", 5)).unwrap()],
    );
    let first = resume.sync(true, 0);
    assert!(first.iter().any(|c| matches!(c, Command::Plex(_))));
    assert!(
        resume
            .sync(true, 1)
            .iter()
            .all(|c| !matches!(c, Command::Plex(_)))
    );
    resume.accept_plex(
        Ok(parse_on_deck(&on_deck(), "http://192.168.1.20:32400").unwrap()),
        10,
    );
    for command in resume.sync(true, 11) {
        if let Command::Image { url, token, .. } = command {
            assert_eq!(
                token.is_some(),
                url.starts_with("http://192.168.1.20"),
                "{url}"
            );
        }
    }
    assert!(
        resume
            .sync(true, 10 + PLEX_REFRESH)
            .iter()
            .any(|c| matches!(c, Command::Plex(_)))
    );
    resume.set_plex(None);
    assert!(
        resume
            .watch_cards()
            .iter()
            .all(|c| c.source != Source::Plex)
    );
    assert!(
        resume
            .sync(true, i64::MAX / 2)
            .iter()
            .all(|c| !matches!(c, Command::Plex(_)))
    );
}
