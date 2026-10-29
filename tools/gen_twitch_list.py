#!/usr/bin/env python3
"""Генерация amnezia_twitch_sites.json для split tunneling AmneziaVPN.

Источники: v2fly/domain-list-community (data/twitch) и маршруты автономной
системы Twitch Interactive AS46489 из RADB.

Отбор курированный: только то, что нужно для работы Twitch (сайт, чат,
плеер, VOD, клипы, расширения). Реклама и сторонняя аналитика исключены.
Google не дублируем, он покрыт amnezia_goog.json.

Важно: AmneziaVPN на маке строит маршруты только из IP и подсетей списка,
сами имена маршрутов не дают. Поэтому имена при генерации резолвятся в /32,
а CloudFront добавляется подсетями из ip-ranges.json AWS. Сайт Twitch стоит
на Fastly, его адреса со временем могут смениться — тогда перегенерировать.
"""
import json
import socket
import urllib.request
from pathlib import Path

# Список Amnezia кладётся в archive/ (вне git); в набор правил Braid его переводит
# tools/amnezia_to_rules.py → state/rules/.
OUT = Path(__file__).resolve().parent.parent / "archive" / "amnezia_twitch_sites.json"

# Домены Twitch. Wildcard до четырёх уровней, потому что видео-эджи имеют
# вид video-edge-xxxxxx.fra05.abs.hls.ttvnw.net
OWNED = [
    "twitch.tv",        # сайт, gql, api, id, passport, чат irc-ws, pubsub
    "ttvnw.net",        # видео: usher, video-weaver, video-edge
    "jtvnw.net",        # статика: аватары, превью, эмоуты (static-cdn)
    "live-video.net",   # Amazon IVS, на нём часть плейлистов и сегментов
    "twitchcdn.net",
    "twitchsvc.net",
    "ext-twitch.tv",    # расширения каналов
    "twitchcon.com",
]
DEPTH = 4

# Явные хосты. Дублируют wildcard, но подстраховывают на случай,
# если сопоставление wildcard в клиенте окажется строже ожидаемого.
EXPLICIT = [
    "www.twitch.tv",
    "m.twitch.tv",
    "gql.twitch.tv",
    "api.twitch.tv",
    "id.twitch.tv",
    "passport.twitch.tv",
    "player.twitch.tv",
    "embed.twitch.tv",
    "clips.twitch.tv",
    "dashboard.twitch.tv",
    "irc-ws.chat.twitch.tv",
    "irc-ws-r.chat.twitch.tv",
    "pubsub-edge.twitch.tv",
    "eventsub.wss.twitch.tv",
    "hermes.twitch.tv",
    "assets.twitch.tv",
    "static.twitchcdn.net",
    "vod-secure.twitch.tv",
    "vod-metro.twitch.tv",
    "vod-storyboards.twitch.tv",
    "clips-media-assets2.twitch.tv",
    "production.assets.clips.twitchcdn.net",
    "usher.ttvnw.net",
    "static-cdn.jtvnw.net",
    "panels-images.twitch.tv",
]

# CloudFront целиком: на нём VOD, клипы, usher и часть видео Twitch.
# Конкретные раздачи меняются, поэтому в туннель идёт весь cloudfront.net:
# имена плюс все его подсети из ip-ranges.json.
CLOUDFRONT = [
    "cloudfront.net",
    "*.cloudfront.net",
    "*.*.cloudfront.net",
]

INFRA = [
    # капча при входе и регистрации
    "arkoselabs.com",
    "*.arkoselabs.com",
]

# Собственные диапазоны Twitch Interactive (AS46489, RADB). Видео-эджи
# живут на них, поэтому ловим поток даже если имя не совпало.
CIDRS = [
    "23.160.0.0/24",
    "45.113.128.0/22",
    "52.129.128.0/17",
    "52.223.192.0/18",
    "99.181.64.0/18",
    "103.53.48.0/22",
    "185.42.204.0/22",
    "192.16.64.0/21",
    "192.108.239.0/24",
    "199.9.248.0/21",
]

AWS_RANGES = "https://ip-ranges.amazonaws.com/ip-ranges.json"


def cloudfront_cidrs() -> list[str]:
    with urllib.request.urlopen(AWS_RANGES, timeout=30) as resp:
        data = json.load(resp)
    return sorted(
        {p["ip_prefix"] for p in data["prefixes"] if p["service"] == "CLOUDFRONT"}
    )


def resolve(host: str) -> list[str]:
    try:
        infos = socket.getaddrinfo(host, 443, socket.AF_INET, socket.SOCK_STREAM)
    except socket.gaierror:
        return []
    return sorted({info[4][0] for info in infos})


def main() -> None:
    entries: list[dict[str, str]] = []
    seen: set[tuple[str, str]] = set()

    def add(hostname: str = "", ip: str = "") -> None:
        key = (hostname, ip)
        if key in seen:
            return
        seen.add(key)
        entries.append({"hostname": hostname, "ip": ip})

    for domain in OWNED:
        add(domain)
        for level in range(1, DEPTH + 1):
            add("*." * level + domain)

    for host in EXPLICIT + CLOUDFRONT + INFRA:
        add(host)

    resolved = {
        f"{ip}/32"
        for host in OWNED + EXPLICIT + INFRA
        if not host.startswith("*")
        for ip in resolve(host)
    }
    cidrs = CIDRS + cloudfront_cidrs() + sorted(resolved)
    # IP и подсети кладём в поле hostname: записи с пустым hostname импорт
    # Amnezia молча выбрасывает (так у Notion потерялась 208.103.161.0/24).
    for cidr in cidrs:
        add(cidr)

    OUT.write_text(json.dumps(entries, indent=1, ensure_ascii=False) + "\n")
    print(f"{OUT.name}: {len(entries)} записей ({len(entries) - len(cidrs)} имён, {len(cidrs)} IP/подсетей)")


if __name__ == "__main__":
    main()
