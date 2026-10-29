/// Тексты для экрана: длительности, даты, названия сетей и движков.
library;

import 'package:braid/models/core_state.dart';

/// «12 мин», «1 ч 05 мин», «2 д 3 ч»; меньше минуты — «только что».
String formatDuration(Duration d) {
  final minutes = d.inMinutes;
  if (minutes < 1) return 'только что';
  if (minutes < 60) return '$minutes мин';
  final hours = d.inHours;
  if (hours < 24) {
    final rest = minutes % 60;
    return rest == 0
        ? '$hours ч'
        : '$hours ч ${rest.toString().padLeft(2, '0')} мин';
  }
  final days = d.inDays;
  final restHours = hours % 24;
  return restHours == 0 ? '$days д' : '$days д $restHours ч';
}

const _months = [
  'янв',
  'фев',
  'мар',
  'апр',
  'мая',
  'июн',
  'июл',
  'авг',
  'сен',
  'окт',
  'ноя',
  'дек',
];

String _two(int n) => n.toString().padLeft(2, '0');

/// «сегодня, 10:15», «вчера, 08:00», «28 окт, 08:00», «28 окт 2025».
/// [local] и [now] — уже в местном времени.
String formatWhen(DateTime local, {required DateTime now}) {
  final time = '${_two(local.hour)}:${_two(local.minute)}';
  final today = DateTime(now.year, now.month, now.day);
  final day = DateTime(local.year, local.month, local.day);
  // Календарная разница: переход на летнее время не сдвигает «вчера».
  final daysAgo = DateTime.utc(
    today.year,
    today.month,
    today.day,
  ).difference(DateTime.utc(day.year, day.month, day.day)).inDays;
  if (daysAgo == 0) return 'сегодня, $time';
  if (daysAgo == 1) return 'вчера, $time';
  final date = '${local.day} ${_months[local.month - 1]}';
  return local.year == now.year ? '$date, $time' : '$date ${local.year}';
}

/// Человекочитаемое имя сети по идентификатору ядра. SSID и MAC в ядро не
/// попадают (там хеш), поэтому название — вид сети и хвост хеша: «Wi‑Fi · 33dd».
String networkLabel(String id) {
  final colon = id.indexOf(':');
  final prefix = colon < 0 ? '' : id.substring(0, colon);
  final tail = id.length <= 4 ? id : id.substring(id.length - 4);
  final kind = prefix == 'wifi' ? 'Wi‑Fi' : 'Сеть';
  return '$kind · $tail';
}

/// Название движка для кнопок и строк: «AWG», «CSQTT»; два движка одного
/// вида различаются идентификатором.
String engineLabel(EngineView engine, List<EngineView> all) {
  final kind = engine.kind.toUpperCase();
  final sameKind = all.where((e) => e.kind == engine.kind).length;
  return sameKind > 1 ? '$kind · ${engine.id}' : kind;
}

/// Причина из ядра называет движок его идентификатором («awg недоступен…») —
/// подменяем на название, как в интерфейсе.
String prettyReason(String reason, List<EngineView> engines) {
  for (final engine in engines) {
    final id = engine.id;
    if (reason.startsWith(id) &&
        (reason.length == id.length || _isSeparator(reason[id.length]))) {
      return engineLabel(engine, engines) + reason.substring(id.length);
    }
  }
  return reason;
}

bool _isSeparator(String char) => char == ' ' || char == ':' || char == ',';

/// Склонение по числу: 1 домен, 2 домена, 5 доменов, 11 доменов, 21 домен.
String ruPlural(int n, String one, String few, String many) {
  final lastTwo = n.abs() % 100;
  final last = n.abs() % 10;
  if (lastTwo >= 11 && lastTwo <= 14) return many;
  if (last == 1) return one;
  if (last >= 2 && last <= 4) return few;
  return many;
}
