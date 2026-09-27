//! IPv4マルチキャストの受信ソケット。

use std::io;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::Duration;

use crate::error::{Context, Result};

/// 受信待ちのタイムアウト。受信スレッドは少なくともこの間隔で停止要求を確認できる。
const RECV_TIMEOUT: Duration = Duration::from_millis(200);

/// `group:port` のマルチキャストを受信するソケットを作る。
pub fn receiver(group: Ipv4Addr, port: u16) -> Result<UdpSocket> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))
        .context(format!("failed to bind UDP port {port}"))?;
    join_all_interfaces(&socket, group)?;
    socket
        .set_read_timeout(Some(RECV_TIMEOUT))
        .context("failed to set socket read timeout")?;
    Ok(socket)
}

/// 1パケット受信する。タイムアウトした場合は `Ok(None)`。
pub fn recv(socket: &UdpSocket, buf: &mut [u8]) -> io::Result<Option<usize>> {
    match socket.recv(buf) {
        Ok(len) => Ok(Some(len)),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

/// マルチキャストグループに、ローカルの全IPv4インターフェースでJoinする。
///
/// インターフェースに `Ipv4Addr::UNSPECIFIED` を渡すと、OSはルーティングメトリックだけで
/// インターフェースを選ぶ。WSL・VPN・Tailscaleなどの仮想アダプタの方がメトリックが低いと
/// そちらでJoinしてしまい、Dante/AES67機器と繋がっている物理NICのパケットが一切届かなくなる。
/// これを避けるため全インターフェースで個別にJoinし、1つでも成功すれば良しとする
/// (ループバックやマルチキャスト非対応のアダプタでの失敗は無視する)。
fn join_all_interfaces(socket: &UdpSocket, group: Ipv4Addr) -> Result<()> {
    let mut joined = false;
    for interface in if_addrs::get_if_addrs().unwrap_or_default() {
        if interface.is_loopback() {
            continue;
        }
        if let IpAddr::V4(addr) = interface.ip() {
            joined |= socket.join_multicast_v4(&group, &addr).is_ok();
        }
    }

    // 列挙に失敗した・1つも成功しなかった場合はOS任せのJoinにフォールバックする。
    if !joined {
        socket
            .join_multicast_v4(&group, &Ipv4Addr::UNSPECIFIED)
            .context(format!("failed to join multicast group {group}"))?;
    }
    Ok(())
}
