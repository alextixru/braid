//! Ввод-вывод tun поверх fd: голый IP и 4-байтный AF-заголовок macOS,
//! пробуждение читателя при закрытии, бесшовная подмена fd.

mod router_common;

use braid_core::testing::MemoryTun;
use braid_core::tun::{TunIo, TunRead};
use router_common::*;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

const AF_INET: [u8; 4] = [0, 0, 0, 2];
const AF_INET6: [u8; 4] = [0, 0, 0, 30];

fn pkt(tag: u8) -> Vec<u8> {
    udp(
        TUN_ADDR,
        1000 + u16::from(tag),
        Ipv4Addr::new(198, 18, 0, 1),
        53,
        &[tag; 3],
    )
}

#[test]
fn reads_and_writes_raw_ip_without_af_header() {
    let (os, fd) = MemoryTun::pair("t0", 1300).unwrap();
    let tun = TunIo::new(fd, false).unwrap();
    assert!(!tun.af_header());
    os.inject(&pkt(1)).unwrap();
    let mut buf = vec![0u8; 2048];
    let TunRead::Packet(n) = tun.read(&mut buf).unwrap() else {
        panic!("ждали пакет");
    };
    assert_eq!(&buf[..n], &pkt(1)[..]);
    tun.write(&pkt(2)).unwrap();
    assert_eq!(os.recv(Duration::from_millis(200)).unwrap(), Some(pkt(2)));
}

#[test]
fn strips_and_adds_af_header_on_macos_style_tun() {
    let (os, fd) = MemoryTun::pair("t1", 1300).unwrap();
    let tun = TunIo::new(fd, true).unwrap();
    assert!(tun.af_header());
    let mut framed = AF_INET.to_vec();
    framed.extend_from_slice(&pkt(1));
    os.inject(&framed).unwrap();
    let mut buf = vec![0u8; 2048];
    let TunRead::Packet(n) = tun.read(&mut buf).unwrap() else {
        panic!("ждали пакет");
    };
    assert_eq!(&buf[..n], &pkt(1)[..], "заголовок снят");

    tun.write(&pkt(2)).unwrap();
    let mut want = AF_INET.to_vec();
    want.extend_from_slice(&pkt(2));
    assert_eq!(
        os.recv(Duration::from_millis(200)).unwrap(),
        Some(want),
        "заголовок поставлен"
    );

    // IPv6 в туннель не идёт (правило 9): кадр пропускается, читатель ждёт следующий.
    let mut v6 = AF_INET6.to_vec();
    v6.extend_from_slice(&[0x60; 40]);
    os.inject(&v6).unwrap();
    let mut framed = AF_INET.to_vec();
    framed.extend_from_slice(&pkt(3));
    os.inject(&framed).unwrap();
    let TunRead::Packet(n) = tun.read(&mut buf).unwrap() else {
        panic!("ждали пакет");
    };
    assert_eq!(&buf[..n], &pkt(3)[..]);
}

#[test]
fn close_wakes_a_blocked_reader() {
    let (os, fd) = MemoryTun::pair("t2", 1300).unwrap();
    let tun = Arc::new(TunIo::new(fd, false).unwrap());
    let reader = {
        let tun = tun.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 2048];
            tun.read(&mut buf).unwrap()
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    tun.close();
    assert!(matches!(reader.join().unwrap(), TunRead::Closed));
    assert!(tun.is_closed());
    assert!(tun.write(&pkt(1)).is_err(), "после закрытия писать некуда");
    assert!(
        os.inject(&pkt(1)).is_err(),
        "fd отпущен: сторона ОС видит закрытого собеседника"
    );
}

#[test]
fn swap_fd_switches_to_the_new_descriptor_without_losing_the_reader() {
    let (os1, fd1) = MemoryTun::pair("t3a", 1300).unwrap();
    let tun = Arc::new(TunIo::new(fd1, false).unwrap());
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let reader = {
        let tun = tun.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 2048];
            loop {
                match tun.read(&mut buf).unwrap() {
                    TunRead::Packet(n) => tx.send(buf[..n].to_vec()).unwrap(),
                    TunRead::Woken => continue,
                    TunRead::Closed => break,
                }
            }
        })
    };
    os1.inject(&pkt(1)).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), pkt(1));

    let (os2, fd2) = MemoryTun::pair("t3b", 1300).unwrap();
    tun.swap_fd(fd2);
    os2.inject(&pkt(2)).unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        pkt(2),
        "читаем новый fd"
    );
    tun.write(&pkt(3)).unwrap();
    assert_eq!(
        os2.recv(Duration::from_millis(200)).unwrap(),
        Some(pkt(3)),
        "пишем в новый fd"
    );
    assert!(
        os1.inject(&pkt(4)).is_err(),
        "старый fd закрыт после подмены"
    );

    tun.close();
    reader.join().unwrap();
    assert!(rx.try_recv().is_err());
}
