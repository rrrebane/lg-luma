use super::*;
use std::net::TcpStream;

fn exchange(request: &str) -> (String, Option<Response>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let request = request.to_owned();
    let client = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        answer
    });
    let (stream, _) = listener.accept().unwrap();
    let response = serve(stream);
    (client.join().unwrap(), response)
}

fn post(path: &str, body: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

const GOOD: &str = r#"{"app":"jupiter","items":[{"id":"1","title":"Film","type":"movie","domain":"jupiter.err.ee","t":30,"d":300,"ts":9}]}"#;

#[test]
fn a_post_from_an_app_is_accepted_with_cors() {
    let (answer, response) = exchange(&post("/continue", GOOD));
    assert!(answer.starts_with("HTTP/1.1 204"), "{answer}");
    assert!(answer.contains("Access-Control-Allow-Origin: *"));
    let Some(Response::Items(Source::Jupiter, items)) = response else {
        panic!("expected a list");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "1");
}

#[test]
fn the_preflight_is_answered_and_other_requests_are_refused() {
    let (answer, response) =
        exchange("OPTIONS /continue HTTP/1.1\r\nHost: x\r\nOrigin: null\r\n\r\n");
    assert!(answer.starts_with("HTTP/1.1 204"));
    assert!(answer.contains("Access-Control-Allow-Headers"));
    assert!(response.is_none());
    let (answer, _) = exchange("GET /continue HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(answer.starts_with("HTTP/1.1 404"));
    let (answer, _) = exchange(&post("/elsewhere", GOOD));
    assert!(answer.starts_with("HTTP/1.1 404"));
}

#[test]
fn bad_bodies_are_rejected() {
    for body in [
        "not json",
        r#"{"app":"netflix","items":[]}"#,
        r#"{"app":"jupiter"}"#,
        r#"{"items":[]}"#,
    ] {
        let (answer, response) = exchange(&post("/continue", body));
        assert!(answer.starts_with("HTTP/1.1 400"), "{body}: {answer}");
        assert!(response.is_none());
    }
    let (answer, _) = exchange(&format!(
        "POST /continue HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY + 1
    ));
    assert!(answer.starts_with("HTTP/1.1 413"));
}
