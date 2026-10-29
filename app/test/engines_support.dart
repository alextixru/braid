/// Подделки для тестов экранов «Движки», «Статистика», «О программе», меню-бара
/// и сервисов VK и капчи: браузер, хранилище, трей, окно, файлы, разрешения.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:braid/screens/stats/stats_service.dart';
import 'package:braid/services/captcha/web_session.dart';
import 'package:braid/services/vk/vk_credentials.dart';
import 'package:braid/tray/notification_permission.dart';
import 'package:braid/tray/system_tray.dart';
import 'package:braid/tray/tray_icon_renderer.dart';
import 'package:braid/tray/tray_status.dart';
import 'package:braid/tray/window_control.dart';
import 'package:flutter/widgets.dart';

/// Страница, которую «видит» пользователь: тест подкладывает адрес и ответы
/// скриптов, а закрывает окно так же, как это сделал бы пользователь.
class FakeWebSession implements WebSession {
  FakeWebSession({this.embeddedWidget});

  final Widget? embeddedWidget;

  /// Адрес текущей страницы; `null` — страница ещё не загрузилась.
  String? url;

  /// Ответы на скрипты по порядку обращений; пока очередь пуста — пустая строка.
  final scriptResults = <String?>[];

  /// Если задан — отвечает вместо очереди.
  String? Function(String script)? evaluator;
  final scripts = <String>[];
  int closeCalls = 0;
  bool throwOnEvaluate = false;
  final _closed = Completer<void>();

  @override
  Widget? get embedded => embeddedWidget;

  @override
  Future<String?> currentUrl() async => url;

  @override
  Future<String?> evaluate(String script) async {
    scripts.add(script);
    if (throwOnEvaluate) throw StateError('страница ещё грузится');
    final answer = evaluator;
    if (answer != null) return answer(script);
    return scriptResults.isEmpty ? '' : scriptResults.removeAt(0);
  }

  @override
  Future<void> get closed => _closed.future;

  /// Пользователь закрыл окно крестиком.
  void userCloses() {
    if (!_closed.isCompleted) _closed.complete();
  }

  @override
  Future<void> close() async {
    closeCalls++;
    userCloses();
  }
}

class FakeWebLauncher implements WebLauncher {
  FakeWebLauncher({this.available = true, FakeWebSession? session})
    : session = session ?? FakeWebSession();

  bool available;
  FakeWebSession session;

  /// Если задан — `open` бросает его.
  Object? openError;
  final requests = <WebRequest>[];

  @override
  Future<bool> isAvailable() async => available;

  @override
  Future<WebSession> open(WebRequest request) async {
    requests.add(request);
    final error = openError;
    if (error != null) throw error;
    return session;
  }
}

/// Хранилище токена и хешей в памяти.
class MemoryVkStore implements VkCredentialStore {
  MemoryVkStore({this.token, List<String>? hashes}) : hashes = [...?hashes];

  String? token;
  List<String> hashes;

  /// Следующая запись упадёт с этой ошибкой (хранилище устройства недоступно).
  Object? writeError;

  @override
  Future<String?> readToken() async => token;

  @override
  Future<void> writeToken(String value) async {
    final error = writeError;
    if (error != null) throw error;
    token = value;
  }

  @override
  Future<void> deleteToken() async => token = null;

  @override
  Future<List<String>> readHashes() async => [...hashes];

  @override
  Future<void> writeHashes(List<String> value) async {
    final error = writeError;
    if (error != null) throw error;
    hashes = [...value];
  }
}

// ---------- меню-бар и окно ----------

/// Значок в строке меню, который всё записывает.
class FakeSystemTray implements SystemTray {
  void Function(String id)? onSelected;
  final icons = <String>[];
  final tooltips = <String>[];
  final menus = <List<TrayMenuItem>>[];
  bool shown = false;
  bool disposed = false;

  /// Следующая запись значка упадёт с этой ошибкой.
  Object? iconError;

  @override
  Future<void> show({required void Function(String id) onSelected}) async {
    shown = true;
    this.onSelected = onSelected;
  }

  @override
  Future<void> setIcon(Uint8List png) async {
    final error = iconError;
    if (error != null) throw error;
    icons.add(utf8.decode(png));
  }

  @override
  Future<void> setTooltip(String text) async => tooltips.add(text);

  @override
  Future<void> setMenu(List<TrayMenuItem> items) async => menus.add(items);

  @override
  Future<void> dispose() async => disposed = true;

  /// Пользователь выбрал пункт меню.
  void select(String id) => onSelected!(id);

  TrayMenuItem item(String id) => menus.last.firstWhere((e) => e.id == id);
}

/// «Картинка» значка — ключ состояния, чтобы тест видел, что нарисовано.
class FakeTrayIcons implements TrayIconRenderer {
  int renders = 0;

  @override
  Future<Uint8List> render(TrayStatus status) async {
    renders++;
    return Uint8List.fromList(utf8.encode(status.iconKey));
  }
}

/// Окно, которое запоминает, что с ним делали, и общий журнал порядка.
class FakeWindowControl implements WindowControl {
  FakeWindowControl([List<String>? log]) : log = log ?? [];

  final List<String> log;
  void Function()? onCloseRequested;
  bool prepared = false;
  int shows = 0;
  int hides = 0;
  int quits = 0;

  /// `prepare` упадёт с этой ошибкой (плагин окна недоступен).
  Object? prepareError;

  @override
  Future<void> prepare({required void Function() onCloseRequested}) async {
    final error = prepareError;
    if (error != null) throw error;
    prepared = true;
    this.onCloseRequested = onCloseRequested;
  }

  @override
  Future<void> show() async {
    shows++;
    log.add('window.show');
  }

  @override
  Future<void> hide() async {
    hides++;
    log.add('window.hide');
  }

  @override
  Future<void> quit() async {
    quits++;
    log.add('window.quit');
  }

  /// Пользователь нажал красную кнопку закрытия окна.
  void userClosesWindow() => onCloseRequested!();
}

// ---------- уведомления ----------

/// Разрешение на уведомления, которое выдаёт «пользователь» в тесте.
class FakeNotificationPermission implements NotificationPermission {
  FakeNotificationPermission({
    this.granted = false,
    this.grantOnRequest = true,
  });

  bool granted;

  /// Что ответит системный запрос.
  bool grantOnRequest;

  /// Системный запрос упадёт (плагина нет, нет Activity).
  Object? requestError;
  int requests = 0;
  int settingsOpened = 0;

  @override
  Future<bool> isGranted() async => granted;

  @override
  Future<bool> request() async {
    requests++;
    final error = requestError;
    if (error != null) throw error;
    granted = grantOnRequest;
    return granted;
  }

  @override
  Future<bool> openSettings() async {
    settingsOpened++;
    return true;
  }
}

class MemoryNotificationPrefs implements NotificationPrefs {
  MemoryNotificationPrefs({this.wasAsked = false});

  bool wasAsked;

  @override
  Future<bool> asked() async => wasAsked;

  @override
  Future<void> setAsked() async => wasAsked = true;
}

// ---------- статистика ----------

/// Файлы экспорта журнала: читает «диск» из памяти, предложение сохранить
/// записывает.
class FakeStatsFiles implements StatsFiles {
  FakeStatsFiles({this.text = ''});

  /// Что лежит в файле экспорта.
  String text;
  final read1 = <String>[];
  final offered = <(String, String)>[];

  /// Куда «сохранил» пользователь; `null` — отказался в диалоге.
  String? savedTo = '/Users/me/braid-journal.jsonl';
  Object? readError;

  @override
  Future<String> read(String path) async {
    read1.add(path);
    final error = readError;
    if (error != null) throw error;
    return text;
  }

  @override
  Future<String?> offer(
    String sourcePath, {
    required String suggestedName,
  }) async {
    offered.add((sourcePath, suggestedName));
    return savedTo;
  }
}

/// Журнал админа, разобранный вручную (те же числа, что в `stats_report_test`):
/// окно 22–29 окт 2026, `now` = 29 окт 12:00 UTC.
///   awg 13 ч 30 мин, csqtt 6 ч → 69 % и 31 %; три переключения.
const statsJournal = '''
{"ts":"2026-10-21T10:00:00Z","event":"connect","engine":"awg","network":"wifi:home"}
{"ts":"2026-10-22T18:00:00Z","event":"network","id":"wifi:cafe","kind":"wifi"}
{"ts":"2026-10-23T00:00:00Z","event":"switch","from":"awg","to":"csqtt","reason":"dead","metrics":{"rtt_ms":null,"loss_pct":100,"handshake_age_s":200}}
{"ts":"2026-10-23T02:00:00Z","event":"probe","engine":"csqtt","rtt_ms":40,"lost":false}
{"ts":"2026-10-23T05:00:00Z","event":"switch","from":"csqtt","to":"awg","reason":"priority","metrics":{"rtt_ms":30,"loss_pct":0,"handshake_age_s":5}}
{"ts":"2026-10-23T06:00:00Z","event":"disconnect","manual":true}
{"ts":"2026-10-28T09:00:00Z","event":"connect","engine":"awg","network":"wifi:home"}
{"ts":"2026-10-28T09:30:00Z","event":"switch","from":"awg","to":"csqtt","reason":"degraded","metrics":{"rtt_ms":2300,"loss_pct":70,"handshake_age_s":20}}
{"ts":"2026-10-28T10:30:00Z","event":"probe","engine":"csqtt","rtt_ms":45,"lost":false}
''';
