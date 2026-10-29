#!/usr/bin/env python3
"""Генерация amnezia_notion_sites.json для split tunneling AmneziaVPN.

Источники: CSP-заголовок app.notion.com (авторитетный перечень разрешённых
эндпоинтов фронтенда), живой захват tcpdump и whois по диапазонам.

Отбор курированный: из CSP берём только то, что нужно для работы Notion.
Реклама, маркетинговая аналитика и соцсети сознательно исключены — иначе
через туннель уйдёт половина веба. Google не дублируем, он покрыт
amnezia_goog.json.
"""
import json
from pathlib import Path

# Список Amnezia кладётся в archive/ (вне git); в набор правил Braid его переводит
# tools/amnezia_to_rules.py → state/rules/.
OUT = Path(__file__).resolve().parent.parent / "archive" / "amnezia_notion_sites.json"

# Домены Notion. Два уровня wildcard, потому что встречаются имена вида
# msgstore-002.app.notion.com
OWNED = [
    "notion.com",
    "notion.so",
    "notion.site",
    "notion.co",
    "notionusercontent.com",
    "notion-static.com",
    "notion.new",
    "notion.ai",
]

# Явные хосты из CSP. Дублируют wildcard, но подстраховывают на случай,
# если сопоставление wildcard в клиенте окажется строже ожидаемого.
EXPLICIT = [
    "app.notion.com",
    "www.notion.com",
    "exp.notion.com",
    "identity.notion.com",
    "identity.notion.so",
    "file.notion.com",
    "file.notion.so",
    "mail.notion.com",
    "mail.notion.so",
    "api.mail.notion.so",
    "mail-resource-proxy.mail.notion.com",
    "calendar.notion.so",
    "realtime.app.notion.com",
    "msgstore.app.notion.com",
    "msgstore-001.app.notion.com",
    "msgstore-002.app.notion.com",
    "msgstore.www.notion.so",
    "msgstore-001.www.notion.so",
    "msgstore-002.www.notion.so",
    "audioprocessor.app.notion.com",
    "audioprocessor.www.notion.so",
    "info.notion.com",
    "library.notion.com",
    "pgncd.notion.com",
    "embed.notion.co",
    "img.notionusercontent.com",
    "artifact.notionusercontent.com",
    "status.notion.so",
]

# S3-бакеты Notion: вложения, временные файлы, эмодзи, снапшоты и бандлы
# кода воркспейсов. Без них ломаются картинки и вложения.
S3 = [
    "prod-files-secure.s3.us-west-2.amazonaws.com",
    "prod-files-secure-apne1.s3.ap-northeast-1.amazonaws.com",
    "prod-files-secure-apne2.s3.ap-northeast-2.amazonaws.com",
    "prod-files-secure-euc1.s3.eu-central-1.amazonaws.com",
    "prod-notion-temporary-files-apne1.s3.ap-northeast-1.amazonaws.com",
    "prod-notion-temporary-files-apne2.s3.ap-northeast-2.amazonaws.com",
    "prod-notion-temporary-files-euc1.s3.eu-central-1.amazonaws.com",
    "prod-space-usw2-0001-workers-code-bundles.s3.us-west-2.amazonaws.com",
    "prod-space-usw2-0002-workers-code-bundles.s3.us-west-2.amazonaws.com",
    "prod-space-usw2-0003-workers-code-bundles.s3.us-west-2.amazonaws.com",
    "prod-space-usw2-0004-workers-code-bundles.s3.us-west-2.amazonaws.com",
    "prod-space-apne1-0001-workers-code-bundles.s3.ap-northeast-1.amazonaws.com",
    "prod-space-apne2-0001-workers-code-bundles.s3.ap-northeast-2.amazonaws.com",
    "prod-space-euc1-0001-workers-code-bundles.s3.eu-central-1.amazonaws.com",
    "notion-emojis.s3-us-west-2.amazonaws.com",
    "notion-production-snapshots-2.s3.us-west-2.amazonaws.com",
    "s3.us-west-2.amazonaws.com",
    "s3-us-west-2.amazonaws.com",
]

# Сторонние сервисы, без которых Notion деградирует или не пускает внутрь.
INFRA = [
    # логи и краши Notion (splunk-хост именной, sentry-организация видна в CSP)
    "http-inputs-notion.splunkcloud.com",
    "o324374.ingest.sentry.io",
    "js.sentry-cdn.com",
    # капча на логине — если заблокировать, вход просто не состоится
    "hcaptcha.com",
    "*.hcaptcha.com",
    "challenges.cloudflare.com",
    # фиче-флаги: без них часть интерфейса не поднимается
    "statsigapi.net",
    "api.statsig.com",
    # поиск
    "8d4f9jvuda.algolia.net",
    # согласие на куки, блокирует рендер баннера
    "transcend-cdn.com",
    "telemetry.transcend.io",
    # контент маркетинговых страниц notion.com
    "images.ctfassets.net",
    "videos.ctfassets.net",
    "cdn.contentful.com",
    # оплата подписки
    "js.stripe.com",
    "checkout.stripe.com",
    "m.stripe.com",
    # js-библиотеки
    "cdnjs.cloudflare.com",
    "cdn.jsdelivr.net",
    # вставки медиа внутрь страниц
    "api.unsplash.com",
    "api.embed.ly",
    "api.giphy.com",
    "*.giphy.com",
]

# Собственный диапазон Notion Labs (whois: NetName NOTION-WEB).
# Ловит всё, что ходит на их адреса, даже если имя не совпало.
CIDRS = ["208.103.161.0/24"]


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
        add(f"*.{domain}")
        add(f"*.*.{domain}")

    for host in EXPLICIT + S3 + INFRA:
        add(host)

    for cidr in CIDRS:
        add(cidr)

    OUT.write_text(json.dumps(entries, indent=1, ensure_ascii=False) + "\n")
    hosts = sum(1 for e in entries if e["hostname"])
    print(f"{OUT.name}: {len(entries)} записей ({hosts} имён, {len(CIDRS)} CIDR)")


if __name__ == "__main__":
    main()
