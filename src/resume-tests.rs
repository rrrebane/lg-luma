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
