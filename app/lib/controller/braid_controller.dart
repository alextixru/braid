/// Контроллер приложения: состояние ядра для экранов и все действия
/// пользователя. Экраны только читают его и зовут методы; сеть, туннель и
/// решения — в ядре за мостом. Состояние — `ChangeNotifier` и поток кадров
/// из Rust, без тяжёлых state-фреймворков (спецификация, «Стек»).
library;

import 'dart:async';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/controller/connect_models.dart';
import 'package:braid/controller/ui_requests.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/models/settings.dart';
import 'package:braid/services/app_dirs.dart';
import 'package:braid/services/clipboard_access.dart';
import 'package:braid/services/deep_links.dart';
import 'package:braid/services/platform_info.dart';
import 'package:braid/services/qr_scanner.dart';
import 'package:braid/services/rules_transfer.dart';
import 'package:braid/services/rules_files.dart';
import 'package:braid/services/ui_prefs.dart';
import 'package:braid/services/url_opener.dart';
import 'package:braid/services/vpn_channel.dart';
import 'package:braid/util/rule_input.dart';
import 'package:flutter/foundation.dart';

class BraidController extends ChangeNotifier {
  BraidController({
    required this.bridge,
    required this.platform,
    this.vpn = const NoVpnChannel(),
    this.clipboard = const SystemClipboard(),
    this.links = const NoDeepLinks(),
    this.prefs = const SharedUiPrefs(),
    this.dirs = const SystemAppDirs(),
    this.files = const NoRulesFiles(),
    this.scanner = const NoQrScanner(),
    this.urls = const SystemUrlOpener(),
    DateTime Function()? now,
    this.serviceStartTimeout = const Duration(seconds: 15),
    this.serviceGrace = const Duration(milliseconds: 2500),
    this.servicePoll = const Duration(milliseconds: 250),
  }) : now = now ?? DateTime.now;

  final BraidBridge bridge;
  final PlatformInfo platform;
  final VpnChannel vpn;
  final ClipboardAccess clipboard;
  final DeepLinks links;
  final UiPrefs prefs;
  final AppDirs dirs;
  final RulesFiles files;
  final QrScanner scanner;
  final UrlOpener urls;

  /// Часы приложения; подменяются в тестах.
  final DateTime Function() now;

  /// Сколько ждать, пока сервис Android построит ядро и начнёт сессию.
  final Duration serviceStartTimeout;

  /// Сколько после запуска сервиса дать ему начать сессию, прежде чем
  /// проверять, не остановился ли он сам (ошибка подключения).
  final Duration serviceGrace;

  /// Как часто проверять, жив ли сервис, пока ждём сессию.
  final Duration servicePoll;

  // ---------- что видят экраны ----------

  StartPhase _phase = StartPhase.starting;
  String? _startError;
  bool _coreReady = false;
  CoreState _state = const CoreState();
  Settings _settings = const Settings();
  RulesCatalog _catalog = const RulesCatalog();
  PlatformCaps _caps = const PlatformCaps(
    perApp: false,
    menuBar: true,
    qrScan: false,
  );
  PrivilegeStatus? _privilege;
  bool _busy = false;
  bool _refreshing = false;
  String? _protocolError;
  DateTime? _connectedSince;
  String? _dismissedUpdate;
  final _dismissedAsk = <String>{};

  // Синхронные: вопрос и сообщение доходят до окна в том же ходе, без гонки
  // с ответом. Слушатель только ставит их в очередь и ничего не вызывает.
  final _requests = StreamController<UiRequest<Object?>>.broadcast(sync: true);
  final _notices = StreamController<Notice>.broadcast(sync: true);

  StartPhase get phase => _phase;
  String? get startError => _startError;

  /// Ядро создано (на Android его строит сервис, позже окна).
  bool get coreReady => _coreReady;
  CoreState get state => _state;
  Settings get settings => _settings;
  RulesCatalog get catalog => _catalog;
  PlatformCaps get caps => _caps;

  /// Правило sudoers (macOS); `null` — не macOS или ещё не спрашивали.
  PrivilegeStatus? get privilege => _privilege;

  /// Идёт `connect()`: кнопка занята.
  bool get busy => _busy;

  /// Идёт ручное обновление подписки.
  bool get refreshing => _refreshing;

  /// Последний кадр ядра не прочитан — текст для экрана.
  String? get protocolError => _protocolError;

  /// Сколько длится подключение — только если переход в «подключено» видели.
  Duration? get connectedFor {
    final since = _connectedSince;
    return since == null ? null : now().difference(since);
  }

  String get clientVersion => bridge.clientVersion();
  String get coreVersion => bridge.coreVersion();

  /// Приложение обновилось: правило sudoers привязано к прошлому помощнику.
  bool get ruleNeedsUpdate {
    final status = _privilege;
    return status != null &&
        status.supported &&
        status.ruleInstalled &&
        !status.helperCurrent;
  }

  /// Баннеры главного экрана без тех, что показаны иначе или закрыты.
  List<CoreBanner> get banners => [
    for (final banner in _state.banners)
      if (!_isHidden(banner)) banner,
  ];

  bool _isHidden(CoreBanner banner) => switch (banner) {
    // Конфликт с другим VPN виден в статусе кнопки, плашка продублировала бы его.
    OtherVpnBanner() => true,
    UpdateAvailableBanner(:final latest) => latest == _dismissedUpdate,
    AutoconnectAskBanner(:final network) => _dismissedAsk.contains(network),
    _ => false,
  };

  /// Вопросы экрану; ответ — через [UiRequest.complete].
  Stream<UiRequest<Object?>> get requests => _requests.stream;

  /// Короткие сообщения.
  Stream<Notice> get notices => _notices.stream;

  // ---------- жизненный цикл ----------

  StreamSubscription<StateFrame>? _frames;
  StreamSubscription<String>? _linkSubscription;
  final Completer<void> _bootstrapped = Completer<void>();
  bool _bootstrapStarted = false;
  int _framesSeen = 0;
  final _readyWaiters = <Completer<void>>[];
  Completer<bool>? _sessionWaiter;
  bool _cancelRequested = false;
  bool _disposed = false;

  /// Запустить ядро (десктоп) или дождаться сервиса (Android), подписаться на
  /// кадры и прочитать всё, что нужно экранам. Повторный вызов после сбоя
  /// пробует снова.
  Future<void> start() async {
    if (_phase == StartPhase.ready || _frames != null && !_failed) return;
    _phase = StartPhase.starting;
    _startError = null;
    _notify();
    _caps = bridge.platformCaps();
    try {
      if (platform.isAndroid) {
        // Ядро строит сервис (контракт 09 ↔ 10): без него моста не к чему
        // подключаться. Просим поднять ядро, но не подключать VPN; сервис,
        // который уже стартовал (загрузка системы), второй раз не трогаем.
        if (!bridge.coreReady() && !await vpn.isServiceRunning()) {
          await vpn.ensureCore();
        }
      } else {
        await bridge.startCore(await dirs.coreDir());
      }
    } on Object catch (e) {
      _fail(describeError(e));
      return;
    }
    _frames ??= bridge.watchState().listen(_onFrame, onError: _onFrameError);
    try {
      await _bootstrapped.future.timeout(serviceStartTimeout);
    } on TimeoutException {
      _fail('Сервис не запустился. Закрой приложение и открой снова.');
    }
  }

  bool get _failed => _phase == StartPhase.failed;

  void _fail(String message) {
    _phase = StartPhase.failed;
    _startError = message;
    _notify();
  }

  void _onFrame(StateFrame frame) {
    final previous = _state;
    _coreReady = frame.ready;
    _state = frame.state;
    _protocolError = null;
    _trackConnection(previous.session, frame.state.session);
    _framesSeen++;
    if (_sessionStarted(frame.state)) {
      final waiter = _sessionWaiter;
      if (waiter != null && !waiter.isCompleted) waiter.complete(true);
    }
    if (frame.ready) {
      for (final waiter in _readyWaiters) {
        if (!waiter.isCompleted) waiter.complete();
      }
      _readyWaiters.clear();
    }
    if (frame.ready && !_bootstrapStarted) {
      _bootstrapStarted = true;
      unawaited(_bootstrap());
    } else if (_bootstrapStarted &&
        _subscriptionChanged(previous, frame.state)) {
      unawaited(_reloadCatalog());
    }
    _notify();
  }

  void _onFrameError(Object error) {
    // Нечитаемый кадр не должен ронять окно: оставляем прежнее состояние
    // и говорим об этом.
    final text = error is FormatException ? error.message : '$error';
    _protocolError = 'Не удалось прочитать состояние ядра: $text';
    _notify();
  }

  bool _subscriptionChanged(CoreState before, CoreState after) {
    return before.subscription.hostId != after.subscription.hostId ||
        before.subscription.version != after.subscription.version;
  }

  void _trackConnection(SessionState before, SessionState after) {
    if (after is! SessionConnected) {
      _connectedSince = null;
    } else if (before is! SessionConnected && _framesSeen > 0) {
      // Приложение открыли на уже идущем подключении — начала не знаем и
      // не выдумываем.
      _connectedSince = now();
    }
  }

  /// Первый кадр с готовым ядром: читаем настройки, каталог, правило sudoers.
  Future<void> _bootstrap() async {
    try {
      await Future.wait([
        bridge.settings().then((value) => _settings = value),
        bridge.rulesCatalog().then((value) => _catalog = value),
        prefs.dismissedUpdate().then((value) => _dismissedUpdate = value),
        _refreshPrivilege(),
      ]);
    } on Object catch (e) {
      // Окно открывается и с умолчаниями; причина видна пользователю.
      _emitNotice(describeError(e));
    }
    _phase = StartPhase.ready;
    _startError = null;
    if (!_bootstrapped.isCompleted) _bootstrapped.complete();
    _notify();
    unawaited(_afterStart());
  }

  /// То, что можно делать, когда окно уже открыто: ссылки и домены из
  /// share-меню. Сессию после перезагрузки восстанавливает само ядро
  /// (`Core::init_with` в фоне, ист. 5) — окно `resume()` не зовёт.
  Future<void> _afterStart() async {
    try {
      _linkSubscription ??= links.links.listen(
        (link) => unawaited(_openLink(link)),
      );
      final initial = await links.initialLink();
      if (initial != null) await _openLink(initial);
      if (platform.isAndroid) await _takeAndroidPending();
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  Future<void> _reloadCatalog() async {
    try {
      _catalog = await bridge.rulesCatalog();
      _notify();
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  /// Окно снова на виду: правило sudoers могли поставить снаружи, а на Android
  /// share-меню и сервис могли оставить домен или ссылку.
  Future<void> onAppResumed() async {
    if (_phase != StartPhase.ready) return;
    unawaited(_refreshPrivilegeQuietly());
    if (platform.isAndroid) await _takeAndroidPending();
  }

  /// Состояние правила — справка для плашки: не прочиталось — остаётся
  /// прежнее, а окну из-за этого шуметь не нужно.
  Future<void> _refreshPrivilegeQuietly() async {
    try {
      await _refreshPrivilege();
    } on Object {
      return;
    }
  }

  Future<void> _takeAndroidPending() async {
    try {
      final link = await vpn.takePendingLink();
      if (link != null) await _openLink(link);
      final domain = await vpn.takeSharedDomain();
      if (domain != null) await addDomainWithConfirmation(domain);
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  Future<PrivilegeStatus?> _refreshPrivilege() async {
    if (!platform.isMacOS) return null;
    final status = await bridge.privilegeStatus();
    _privilege = status;
    _notify();
    return status;
  }

  // ---------- подключение ----------

  /// Подключиться. Ошибки показываются сообщением; состояние приходит кадрами.
  Future<void> connect() async {
    if (_busy) return;
    _busy = true;
    _cancelRequested = false;
    _notify();
    try {
      if (platform.isAndroid) {
        await _connectAndroid();
      } else {
        await _offerSudoersOnFirstConnect();
        // «Отключить» из меню-бара, пока шёл вопрос о правиле, — не подключаемся.
        if (_cancelRequested) return;
        await bridge.connect();
      }
    } on Object catch (e) {
      _emitNotice(describeError(e));
    } finally {
      _busy = false;
      _notify();
    }
  }

  /// Android: согласие VPN, затем либо прямой `Core::connect` (сервис уже
  /// привязан к ядру), либо запуск сервиса — он сам зовёт `Core::connect`
  /// после привязки к ядру. Звать `connect()` из окна одновременно с запуском
  /// нельзя: он гонится с привязкой платформы к новому экземпляру сервиса.
  Future<void> _connectAndroid() async {
    if (!await vpn.prepare()) {
      _emitNotice('Без разрешения VPN подключиться нельзя');
      return;
    }
    if (_coreReady && await vpn.isServiceRunning()) {
      await bridge.connect();
      return;
    }
    await vpn.startService();
    if (!_coreReady && !await _waitForCore()) {
      _emitNotice('Сервис не запустился. Закрой приложение и открой снова.');
      return;
    }
    if (!await _awaitSession() && !_cancelRequested) {
      _emitNotice(
        'Не удалось подключиться. Подробности — в уведомлении Braid.',
      );
    }
  }

  bool _sessionStarted(CoreState state) =>
      state.session is! SessionIdle && state.session is! SessionNeedsLink;

  /// Ждёт, пока ядро начнёт сессию после запуска сервиса. `false` — сервис
  /// остановился сам (не удалось подключиться), время вышло или отмена.
  Future<bool> _awaitSession() async {
    if (_sessionStarted(_state)) return true;
    final waiter = Completer<bool>();
    _sessionWaiter = waiter;
    var waited = Duration.zero;
    final poll = Timer.periodic(servicePoll, (_) async {
      if (waiter.isCompleted) return;
      waited += servicePoll;
      if (waited >= serviceGrace && !await vpn.isServiceRunning()) {
        if (!waiter.isCompleted) waiter.complete(false);
      }
    });
    try {
      return await waiter.future.timeout(
        serviceStartTimeout,
        onTimeout: () => false,
      );
    } finally {
      poll.cancel();
      if (identical(_sessionWaiter, waiter)) _sessionWaiter = null;
    }
  }

  /// Ждёт, пока сервис построит ядро. `false` — не дождались за
  /// [serviceStartTimeout].
  Future<bool> _waitForCore() async {
    if (_coreReady) return true;
    final waiter = Completer<void>();
    _readyWaiters.add(waiter);
    try {
      await waiter.future.timeout(serviceStartTimeout);
    } on TimeoutException {
      _readyWaiters.remove(waiter);
    }
    return _coreReady;
  }

  Future<void> disconnect() async {
    _cancelRequested = true;
    final waiter = _sessionWaiter;
    if (waiter != null && !waiter.isCompleted) waiter.complete(false);
    try {
      await bridge.disconnect();
      if (platform.isAndroid) await vpn.stopService();
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  /// Кнопка главного экрана: по состоянию подключает или отключает.
  Future<void> toggleConnection() {
    // Идёт запуск сессии: та же кнопка — «Отменить».
    if (_busy) return disconnect();
    return switch (_state.session) {
      SessionConnected() || SessionConnecting() => disconnect(),
      _ => connect(),
    };
  }

  // ---------- правило sudoers (macOS) ----------

  /// Первое подключение: предложить «Не спрашивать пароль в дальнейшем».
  /// Отказ запоминается — дальше только из настроек. Сбой установки не
  /// мешает подключению: пароль спросят, как раньше.
  Future<void> _offerSudoersOnFirstConnect() async {
    if (!platform.isMacOS) return;
    try {
      final status = await _refreshPrivilege();
      if (status == null || !status.supported) return;
      // Правило стоит: если помощник устарел, об этом скажет баннер, а не подключение.
      if (status.ruleInstalled) return;
      if (await prefs.sudoersOfferDeclined()) return;
      final plan = await bridge.planSudoersInstall();
      final answer = await _ask(OfferSudoers(plan: plan), SudoersAnswer.later);
      if (answer == SudoersAnswer.later) {
        await prefs.setSudoersOfferDeclined(true);
        return;
      }
      await _applySudoers(plan);
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  /// Поставить или обновить правило (настройки, баннер «Обновить правило»):
  /// пользователь видит ту же команду, на которую соглашается.
  Future<void> installSudoersRule() async {
    try {
      final status = await _refreshPrivilege();
      if (status == null || !status.supported) {
        _emitNotice('Правило sudoers есть только на macOS');
        return;
      }
      final plan = await bridge.planSudoersInstall();
      final answer = await _ask(
        OfferSudoers(plan: plan, update: status.ruleInstalled),
        SudoersAnswer.later,
      );
      if (answer == SudoersAnswer.install) await _applySudoers(plan);
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  Future<void> _applySudoers(SudoersPlan plan) async {
    try {
      await bridge.applySudoersInstall(plan.command);
      _emitNotice('Готово: пароль администратора больше не понадобится');
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
    try {
      await _refreshPrivilege();
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  // ---------- подписка ----------

  /// Ссылка из поля, QR или deeplink. Ссылка другого хоста при живой подписке
  /// спрашивает замену. Ошибка возвращается текстом — экран покажет её на
  /// месте, не стирая ввод.
  Future<ImportResult> importLink(String text) async {
    final link = text.trim();
    if (link.isEmpty) {
      return const ImportFailed('Сначала вставь ссылку или отсканируй QR');
    }
    try {
      final outcome = await bridge.importLink(link);
      switch (outcome) {
        case Imported():
          return const ImportDone();
        case ReplaceRequested(:final currentHost, :final newHost):
          final confirmed = await _ask(
            ConfirmReplace(currentHost: currentHost, newHost: newHost),
            false,
          );
          if (!confirmed) return const ImportCancelled();
          await bridge.replaceLink(link);
          return const ImportDone();
      }
    } on Object catch (e) {
      return ImportFailed(describeError(e));
    }
  }

  Future<void> _openLink(String link) async {
    if (!link.trim().toLowerCase().startsWith('braid:')) return;
    switch (await importLink(link)) {
      case ImportDone():
        _emitNotice('Подписка добавлена');
      case ImportCancelled():
        break;
      case ImportFailed(:final message):
        _emitNotice(message);
    }
  }

  /// «Обновить сейчас».
  Future<void> refreshSubscription() async {
    if (_refreshing) return;
    _refreshing = true;
    _notify();
    try {
      final outcome = await bridge.refreshSubscription();
      _emitNotice(switch (outcome) {
        Updated(:final version) => 'Подписка обновлена · версия $version',
        NotModified() => 'Изменений нет — подписка актуальна',
      });
    } on Object catch (e) {
      _emitNotice(describeError(e));
    } finally {
      _refreshing = false;
      _notify();
    }
  }

  /// Ссылка текущей подписки — для QR и копирования. Секрет: только по
  /// действию пользователя.
  Future<String?> subscriptionLink() async {
    try {
      return await bridge.subscriptionLink();
    } on Object catch (e) {
      _emitNotice(describeError(e));
      return null;
    }
  }

  /// Показать короткое сообщение внизу экрана (из обработчиков экранов).
  void showNotice(String message) => _emitNotice(message);

  /// Открыть ссылку в браузере (сборка обновления из подписки).
  Future<void> openUrl(String url) async {
    try {
      if (await urls.open(url)) return;
    } on Object catch (e) {
      _emitNotice(describeError(e));
      return;
    }
    _emitNotice('Не удалось открыть ссылку');
  }

  /// Скопировать ссылку подписки в буфер обмена (ист. 36).
  Future<void> copySubscriptionLink() async {
    final link = await subscriptionLink();
    if (link == null) {
      _emitNotice('Ссылка подписки недоступна');
      return;
    }
    try {
      await clipboard.writeText(link);
      _emitNotice('Ссылка скопирована');
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  Future<void> dismissUpdate(String version) async {
    _dismissedUpdate = version;
    _notify();
    await prefs.setDismissedUpdate(version);
  }

  // ---------- правила ----------

  /// Отправить изменение правил; `null` — принято, иначе текст ошибки.
  Future<String?> applyRules(RulesChange change) async {
    try {
      await bridge.setRules(change);
      return null;
    } on Object catch (e) {
      return describeError(e);
    }
  }

  /// «Мои правила»: домен или подсеть одним полем; `null` — добавлено.
  Future<String?> addRule(String input) async {
    final NormalizedRule rule;
    try {
      rule = RuleInput.classify(input);
    } on RuleInputException catch (e) {
      return e.message;
    }
    return applyRules(switch (rule.kind) {
      RuleKind.domain => RulesChange.addUserDomain(rule.value),
      RuleKind.subnet => RulesChange.addUserCidr(rule.value),
    });
  }

  /// «+ Домен»: домен из буфера обмена с подтверждением.
  Future<void> addDomainFromClipboard() async {
    final String? text;
    try {
      text = await clipboard.readText();
    } on Object catch (e) {
      _emitNotice(describeError(e));
      return;
    }
    final domain = text == null ? null : RuleInput.domainFromText(text);
    if (domain == null) {
      _emitNotice('В буфере нет ссылки или домена');
      return;
    }
    await addDomainWithConfirmation(domain);
  }

  /// Спросить и добавить домен (буфер обмена, share-меню).
  Future<void> addDomainWithConfirmation(String domain) async {
    if (_state.rules.userDomains.contains(domain)) {
      _emitNotice('$domain уже в твоих правилах');
      return;
    }
    if (!await _ask(ConfirmDomain(domain), false)) return;
    final error = await applyRules(RulesChange.addUserDomain(domain));
    _emitNotice(error ?? 'Добавлено: $domain');
  }

  /// Сохранить свои домены и подсети в файл (десктоп, ист. 52).
  Future<void> exportUserRules() async {
    if (!files.available) return;
    try {
      final text = RulesTransfer.encode(
        domains: _state.rules.userDomains,
        subnets: _state.rules.userCidrs,
      );
      final saved = await files.save(RulesTransfer.fileName, text);
      if (saved != null) _emitNotice('Сохранено: $saved');
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  /// Добавить правила из файла. Плохие записи пропускаются, итог — числами.
  Future<void> importUserRules() async {
    if (!files.available) return;
    try {
      final text = await files.open();
      if (text == null) return;
      final plan = RulesTransfer.decode(text);
      var added = 0;
      var skipped = plan.rejected.length;
      final changes = [
        for (final domain in plan.domains) RulesChange.addUserDomain(domain),
        for (final subnet in plan.subnets) RulesChange.addUserCidr(subnet),
      ];
      for (final change in changes) {
        if (await applyRules(change) == null) {
          added++;
        } else {
          skipped++;
        }
      }
      _emitNotice(
        skipped == 0
            ? 'Добавлено: $added'
            : 'Добавлено: $added · пропущено: $skipped',
      );
    } on FormatException catch (e) {
      _emitNotice(e.message);
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  Future<void> setEngineMode(EngineMode mode) async {
    try {
      await bridge.setEngineMode(mode);
    } on Object catch (e) {
      _emitNotice(describeError(e));
    }
  }

  // ---------- настройки ----------

  /// Изменить настройки: видно сразу, отказ ядра откатывает и объясняет.
  Future<void> updateSettings(
    Settings Function(Settings current) change,
  ) async {
    final previous = _settings;
    final next = change(previous);
    if (next == previous) return;
    _settings = next;
    _notify();
    try {
      await bridge.setSettings(next);
    } on Object catch (e) {
      _settings = previous;
      _notify();
      _emitNotice(describeError(e));
      await _restoreSettings(previous);
    }
  }

  /// Ядро могло сохранить новое значение до отказа (автозапуск не включился,
  /// а выбор записан) — возвращаем ему прежнее, чтобы переключатель не врал.
  /// Не получилось и это — остаётся то, что показано, ошибка уже сообщена.
  Future<void> _restoreSettings(Settings previous) async {
    try {
      await bridge.setSettings(previous);
    } on Object {
      return;
    }
  }

  /// Ответ на «Новая сеть — подключаться здесь?».
  Future<void> answerAutoconnect(
    String network,
    AutoconnectAnswer answer,
  ) async {
    _dismissedAsk.add(network);
    _notify();
    switch (answer) {
      case AutoconnectAnswer.once:
        break;
      case AutoconnectAnswer.always:
        await updateSettings(
          (s) => _withNetwork(s, network, Autoconnect.connect),
        );
      case AutoconnectAnswer.never:
        await updateSettings(
          (s) => _withNetwork(s, network, Autoconnect.never),
        );
        return;
    }
    await connect();
  }

  Settings _withNetwork(Settings settings, String network, Autoconnect rule) {
    return settings.copyWith(
      autoconnectNetworks: {...settings.autoconnectNetworks, network: rule},
    );
  }

  // ---------- для экранов тикета 11 ----------

  Future<List<EngineProbeReport>> probeNow() => bridge.probeNow();

  Future<void> userInput(String engine, UserInput input) =>
      bridge.userInput(engine, input);

  Future<String> exportStats() => bridge.exportStats();

  Future<List<String>> recentErrors() => bridge.recentErrors();

  // ---------- общее ----------

  /// Текст ошибки для экрана: у ошибок моста он уже по-русски.
  static String describeError(Object error) {
    return switch (error) {
      BridgeError(:final message) => message,
      FormatException(:final message) =>
        'Не удалось прочитать ответ ядра: $message',
      _ => 'Внутренняя ошибка: $error',
    };
  }

  Future<T> _ask<T>(UiRequest<T> request, T fallback) {
    // Некому показать диалог (окна нет) — действуем осторожно.
    if (!_requests.hasListener) return Future.value(fallback);
    _requests.add(request);
    return request.answer;
  }

  void _emitNotice(String message) {
    if (!_notices.isClosed) _notices.add(Notice(message));
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(_frames?.cancel());
    unawaited(_linkSubscription?.cancel());
    unawaited(_requests.close());
    unawaited(_notices.close());
    super.dispose();
  }
}
