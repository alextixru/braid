//! Контрольные суммы IPv4/TCP/UDP/ICMP (RFC 1071) и инкрементальный
//! пересчёт при подмене полей (RFC 1624, формула 3): `HC' = ~(~HC + ~m + m')`.

use std::net::Ipv4Addr;

/// Сумма 16-битных слов в дополнении до единицы, без свёртки переносов.
pub fn sum(data: &[u8]) -> u32 {
    let (words, rest) = data.as_chunks::<2>();
    let mut acc: u32 = words
        .iter()
        .map(|w| u32::from(u16::from_be_bytes(*w)))
        .sum();
    if let [last] = rest {
        acc += u32::from(u16::from_be_bytes([*last, 0]));
    }
    acc
}

/// Свернуть переносы в 16 бит.
pub fn fold(mut acc: u32) -> u16 {
    while acc >> 16 != 0 {
        acc = (acc & 0xffff) + (acc >> 16);
    }
    acc as u16
}

/// Готовая контрольная сумма данных (поле суммы внутри должно быть нулевым).
pub fn finish(acc: u32) -> u16 {
    !fold(acc)
}

/// Сумма заголовка IPv4: поле checksum считается нулевым.
pub fn ipv4(header: &[u8]) -> u16 {
    let acc = sum(&header[..10]) + sum(&header[12..]);
    finish(acc)
}

fn pseudo(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, len: usize) -> u32 {
    sum(&src.octets()) + sum(&dst.octets()) + u32::from(proto) + (len as u32 & 0xffff)
}

/// Сумма TCP/UDP с псевдозаголовком; `csum_at` — смещение поля суммы в
/// сегменте, оно считается нулевым.
pub fn transport(src: Ipv4Addr, dst: Ipv4Addr, proto: u8, segment: &[u8], csum_at: usize) -> u16 {
    let acc = pseudo(src, dst, proto, segment.len())
        + sum(&segment[..csum_at])
        + sum(&segment[csum_at + 2..]);
    finish(acc)
}

/// Сумма ICMP (без псевдозаголовка); поле суммы в байтах 2..4 считается нулевым.
pub fn icmp(message: &[u8]) -> u16 {
    finish(sum(&message[..2]) + sum(&message[4..]))
}

/// Инкрементальный пересчёт: в данных слова `old` заменились на `new`
/// (одинаковой чётной длины).
pub fn update(old_csum: u16, old: &[u8], new: &[u8]) -> u16 {
    debug_assert_eq!(old.len(), new.len());
    let mut acc = u32::from(!old_csum);
    for w in old.as_chunks::<2>().0 {
        acc += u32::from(!u16::from_be_bytes(*w));
    }
    acc += sum(new);
    !fold(acc)
}

/// Проверка: сумма блока с заполненным полем суммы равна нулю.
pub fn verify(data: &[u8]) -> bool {
    fold(sum(data)) == 0xffff
}
