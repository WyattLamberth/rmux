use rmux::adapters::metrics::{Metrics, MetricsServer};
use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::Arc,
    time::Duration,
};

#[test]
fn local_metrics_endpoint_and_shutdown() {
    let server =
        MetricsServer::start("127.0.0.1:0".parse().unwrap(), Arc::new(Metrics::default())).unwrap();
    let address = server.address;
    let request = |path: &str| {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    };
    let response = request("/metrics");
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains("rmux_panes 0"));
    assert!(request("/").starts_with("HTTP/1.1 404 Not Found"));
    let mut fragmented = TcpStream::connect(address).unwrap();
    fragmented
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    fragmented.write_all(b"GE").unwrap();
    std::thread::sleep(Duration::from_millis(10));
    fragmented
        .write_all(b"T /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    fragmented.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    drop(server);
    assert!(
        TcpStream::connect(address).is_err(),
        "metrics listener leaked"
    );
}
