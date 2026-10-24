use crate::common::get_arg;
use hbb_common::{
    anyhow::Context,
    tokio::{net::TcpStream, time::{timeout, Duration}},
    ResultType,
};
use ipnetwork::IpNetwork;
use std::net::{IpAddr, SocketAddr};
use tokio_tungstenite::{
    tungstenite::{handshake::server::{Request, Response}, http::HeaderMap},
    WebSocketStream,
};

pub async fn accept(
    stream: TcpStream,
    addr: SocketAddr,
) -> ResultType<(WebSocketStream<TcpStream>, SocketAddr)> {
    let trusted_proxies = parse_trusted_proxies(&get_arg("TRUSTED_PROXIES"))?;
    let mut peer_addr = addr;
    let callback = |request: &Request, response: Response| {
        peer_addr = forwarded_addr(addr, request.headers(), &trusted_proxies);
        Ok(response)
    };
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(crate::common::MAX_CONTROL_MESSAGE))
        .max_frame_size(Some(crate::common::MAX_CONTROL_MESSAGE));
    let stream = timeout(
        Duration::from_secs(10),
        tokio_tungstenite::accept_hdr_async_with_config(stream, callback, Some(config)),
    )
    .await??;
    Ok((stream, peer_addr))
}

// Called at startup so a bad TRUSTED_PROXIES fails fast instead of breaking every handshake.
pub fn validate_config() -> ResultType<()> {
    parse_trusted_proxies(&get_arg("TRUSTED_PROXIES")).map(|_| ())
}

fn parse_trusted_proxies(value: &str) -> ResultType<Vec<IpNetwork>> {
    value.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::parse::<IpNetwork>)
        .collect::<Result<Vec<_>, _>>()
        .context("Invalid TRUSTED_PROXIES: expected IP addresses or CIDR networks")
}

fn forwarded_addr(addr: SocketAddr, headers: &HeaderMap, trusted: &[IpNetwork]) -> SocketAddr {
    let is_trusted = |ip: IpAddr| {
        let ip = hbb_common::try_into_v4(SocketAddr::new(ip, 0)).ip();
        trusted.iter().any(|network| network.contains(ip))
    };
    if !is_trusted(addr.ip()) {
        return addr;
    }
    if let Some(value) = headers.get("X-Real-IP") {
        return value
            .to_str()
            .ok()
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
            .map(|ip| SocketAddr::new(ip, 0))
            .unwrap_or(addr);
    }
    if headers.contains_key("X-Forwarded-For") {
        let mut ip = addr.ip();
        // Walk all header lines from the nearest proxy; clients can forge earlier entries.
        for value in headers.get_all("X-Forwarded-For").iter().rev() {
            if !is_trusted(ip) {
                return SocketAddr::new(ip, 0);
            }
            let Ok(value) = value.to_str() else { return addr };
            for value in value.split(',').rev() {
                if !is_trusted(ip) {
                    return SocketAddr::new(ip, 0);
                }
                match value.trim().parse::<IpAddr>() {
                    Ok(value) => ip = value,
                    Err(_) => return addr,
                }
            }
        }
        return SocketAddr::new(ip, 0);
    }
    addr
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio;

    #[test]
    fn invalid_trusted_proxy_configuration_is_rejected() {
        assert!(parse_trusted_proxies("").unwrap().is_empty());
        assert_eq!(parse_trusted_proxies(" 127.0.0.1/32, ::1/128 ").unwrap().len(), 2);
        assert!(parse_trusted_proxies("127.0.0.1/32,not-a-network").is_err());
    }

    #[test]
    fn untrusted_peers_cannot_spoof_forwarded_addresses() {
        let addr = "203.0.113.10:1234".parse().unwrap();
        let request = Request::builder()
            .header("X-Real-IP", "192.0.2.1")
            .header("X-Forwarded-For", "192.0.2.2")
            .body(())
            .unwrap();
        assert_eq!(forwarded_addr(addr, request.headers(), &[]), addr);
        let trusted = ["127.0.0.0/8".parse().unwrap()];
        assert_eq!(forwarded_addr(addr, request.headers(), &trusted), addr);
    }

    #[test]
    fn trusted_proxy_addresses_support_ipv4_and_ipv6() {
        let trusted = ["127.0.0.1/32".parse().unwrap(), "::1/128".parse().unwrap()];
        for proxy in ["127.0.0.1:1234", "[::1]:1234", "[::ffff:127.0.0.1]:1234"] {
            for client in ["192.0.2.1", "2001:db8::1"] {
                let request = Request::builder().header("X-Real-IP", client).body(()).unwrap();
                let actual = forwarded_addr(proxy.parse().unwrap(), request.headers(), &trusted);
                assert_eq!(actual, SocketAddr::new(client.parse().unwrap(), 0));
            }
        }
    }

    #[test]
    fn forwarded_chain_stops_at_the_first_untrusted_hop() {
        let addr = "127.0.0.1:1234".parse().unwrap();
        let trusted = ["127.0.0.1/32".parse().unwrap(), "198.51.100.0/24".parse().unwrap()];
        for values in [
            vec!["192.0.2.99, 203.0.113.10, 198.51.100.2"],
            vec!["192.0.2.99", "203.0.113.10, 198.51.100.2"],
        ] {
            let mut request = Request::builder();
            for value in values {
                request = request.header("X-Forwarded-For", value);
            }
            let request = request.body(()).unwrap();
            assert_eq!(
                forwarded_addr(addr, request.headers(), &trusted),
                "203.0.113.10:0".parse().unwrap(),
            );
        }
    }

    #[test]
    fn missing_or_malformed_forwarded_addresses_keep_the_socket_address() {
        let addr = "127.0.0.1:1234".parse().unwrap();
        let trusted = ["127.0.0.1/32".parse().unwrap()];
        let empty = Request::builder().body(()).unwrap();
        assert_eq!(forwarded_addr(addr, empty.headers(), &trusted), addr);
        for name in ["X-Real-IP", "X-Forwarded-For"] {
            let request = Request::builder().header(name, "not-an-ip").body(()).unwrap();
            assert_eq!(forwarded_addr(addr, request.headers(), &trusted), addr);
        }
    }

    #[hbb_common::tokio::test]
    async fn websocket_handshake_preserves_binary_messages() {
        use hbb_common::{futures_util::{SinkExt, StreamExt}, tokio::net::TcpListener};
        use tokio_tungstenite::tungstenite::Message;

        timeout(Duration::from_secs(5), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = hbb_common::tokio::spawn(async move {
                let (stream, addr) = listener.accept().await.unwrap();
                let (mut socket, peer_addr) = accept(stream, addr).await.unwrap();
                assert_eq!(peer_addr, addr);
                let message = socket.next().await.unwrap().unwrap();
                socket.send(message).await.unwrap();
            });
            let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
                .await.unwrap();
            let payload = Message::Binary(vec![0, 1, 127, 255].into());
            client.send(payload.clone()).await.unwrap();
            assert_eq!(client.next().await.unwrap().unwrap(), payload);
            server.await.unwrap();
        }).await.unwrap();
    }

    #[hbb_common::tokio::test]
    async fn oversized_websocket_control_messages_are_rejected() {
        use hbb_common::{futures_util::{SinkExt, StreamExt}, tokio::net::TcpListener};
        use tokio_tungstenite::tungstenite::{Error, Message, protocol::frame::{Frame, coding::{Data, OpCode}}};
        for fragmented in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, addr) = listener.accept().await.unwrap();
                let (mut socket, _) = accept(stream, addr).await.unwrap();
                assert!(matches!(socket.next().await.unwrap(), Err(Error::Capacity(_))));
            });
            let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}")).await.unwrap();
            if fragmented {
                let part = vec![0; crate::common::MAX_CONTROL_MESSAGE / 2 + 1];
                client.send(Message::Frame(Frame::message(part.clone(), OpCode::Data(Data::Binary), false))).await.unwrap();
                let _ = client.send(Message::Frame(Frame::message(part, OpCode::Data(Data::Continue), true))).await;
            } else {
                let _ = client.send(Message::Binary(vec![0; crate::common::MAX_CONTROL_MESSAGE + 1].into())).await;
            }
            timeout(Duration::from_secs(3), server).await.unwrap().unwrap();
        }
    }

    #[hbb_common::tokio::test]
    async fn stalled_websocket_handshakes_time_out() {
        use hbb_common::tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (stream, addr) = listener.accept().await.unwrap();
        let error = accept(stream, addr).await.unwrap_err();
        assert!(error.downcast_ref::<hbb_common::tokio::time::error::Elapsed>().is_some());
        drop(client);
    }
}
