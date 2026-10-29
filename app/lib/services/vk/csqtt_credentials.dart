/// Хеши звонков и вход через VK для движка CSQTT (ист. 14–17, 78).
///
/// Ядро держит хеши и токен только в памяти движка: до выхода из приложения
/// они сохраняются между сессиями, после перезапуска — нет. Поэтому их хранит
/// экран (защищённое хранилище устройства) и возвращает движку, когда тот
/// просит: событием `NeedsHashes` → `EngineView.user_action = need_hashes`.
/// Возвращаем не чаще раза за сессию: если ядро снова просит хеши, значит
/// сохранённые не сработали (звонки протухли), и крутить их по кругу нельзя —
/// пользователю нужны новые.
library;

import 'dart:async';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/services/captcha/web_flow.dart';
import 'package:braid/services/vk/call_hash.dart';
import 'package:braid/services/vk/vk_credentials.dart';
import 'package:braid/services/vk/vk_oauth.dart';
import 'package:flutter/foundation.dart';

class CsqttCredentials extends ChangeNotifier {
  CsqttCredentials({
    required this.controller,
    required this.store,
    required this.oauth,
  });

  final BraidController controller;
  final VkCredentialStore store;
  final VkOAuth oauth;

  /// `EngineError::NotStarted` ядра: сессии с этим движком ещё не было, слот
  /// для него появляется при первом подключении.
  static const _engineNotStarted = 'движок не запущен';

  List<String> _hashes = const [];
  String? _token;
  bool _tokenPersisted = true;
  bool _loaded = false;
  bool _attached = false;
  bool _disposed = false;

  // Сессия ядра: номер растёт при каждом новом подключении; сохранённое
  // отдаётся движку не больше одного раза на номер.
  bool _sessionActive = false;
  int _epoch = 0;
  int _suppliedEpoch = -1;
  bool _wasRejected = false;

  /// Хеши звонков в порядке ввода (нормализованные).
  List<String> get hashes => _hashes;

  /// Вход через VK выполнен: токен есть.
  bool get loggedIn => _token != null;

  /// Токен лежит только в памяти: защищённое хранилище устройства его не
  /// приняло, при выходе из приложения вход пропадёт.
  bool get tokenPersisted => _tokenPersisted;

  /// Хранилище прочитано.
  bool get loaded => _loaded;

  bool get hasCredentials => _token != null || _hashes.isNotEmpty;

  /// Движок CSQTT из подписки; `null` — в подписке его нет.
  EngineView? get engine {
    for (final engine in controller.state.engines) {
      if (engine.kind == 'csqtt') return engine;
    }
    return null;
  }

  /// Ядро просит войти заново: токен VK не принят.
  bool get tokenRejected =>
      _token != null && engine?.userAction is VkLoginAction;

  Future<void>? _loading;

  /// Прочитать хранилище и начать следить за состоянием ядра. Повторный и
  /// одновременный вызовы (запуск ядра и открытие экрана) читают один раз.
  Future<void> load() => _loading ??= _load();

  Future<void> _load() async {
    try {
      _hashes = await store.readHashes();
    } on Object {
      _hashes = const [];
    }
    try {
      _token = await store.readToken();
    } on Object {
      _token = null;
    }
    _loaded = true;
    _attach();
    _changed();
    _supplyIfAsked();
  }

  void _attach() {
    if (_attached) return;
    _attached = true;
    _sessionActive = _isActive(controller.state.session);
    _epoch = _sessionActive ? 1 : 0;
    controller.addListener(_onController);
  }

  @override
  void dispose() {
    _disposed = true;
    if (_attached) controller.removeListener(_onController);
    super.dispose();
  }

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  // ---------- хеши ----------

  /// Добавить звонок. `null` — принято; иначе текст ошибки для экрана (ввод
  /// при этом не стирается).
  Future<String?> addHash(String input) async {
    final hash = parseCallHash(input);
    if (hash == null) return kInvalidCallLink;
    if (_hashes.contains(hash)) return 'Этот звонок уже в списке';
    final previous = _hashes;
    final next = [...previous, hash];
    try {
      await store.writeHashes(next);
    } on Object {
      return 'Не удалось сохранить хеш в защищённом хранилище устройства';
    }
    _hashes = next;
    _changed();
    _suppliedEpoch = _epoch;
    final rejected = await _send(UserInput.vkHashes(next));
    if (rejected != null) {
      _hashes = previous;
      await _writeQuietly(previous);
      _changed();
      return rejected;
    }
    return null;
  }

  Future<void> removeHash(String hash) async {
    if (!_hashes.contains(hash)) return;
    final next = [
      for (final item in _hashes)
        if (item != hash) item,
    ];
    _hashes = next;
    _changed();
    await _writeQuietly(next);
    _suppliedEpoch = _epoch;
    await _send(UserInput.vkHashes(next));
  }

  Future<void> _writeQuietly(List<String> hashes) async {
    try {
      await store.writeHashes(hashes);
    } on Object {
      // Список на экране уже верный; хранилище поправится при следующей записи.
    }
  }

  // ---------- вход через VK ----------

  /// Открыть окно входа. Показ окна и ожидание — на экране; итог передаётся
  /// в [completeLogin].
  Future<WebFlow<VkLoginResult>> beginLogin() => oauth.begin();

  /// Итог входа. `null` — вошли; иначе текст для экрана (отказ или
  /// предупреждение, что вход не переживёт перезапуск).
  Future<String?> completeLogin(VkLoginResult result) async {
    switch (result) {
      case VkLoginFailed(:final message):
        return message;
      case VkLoginSucceeded(:final token):
        _token = token;
        try {
          await store.writeToken(token);
          _tokenPersisted = true;
        } on Object {
          _tokenPersisted = false;
        }
        _changed();
        _suppliedEpoch = _epoch;
        final rejected = await _send(UserInput.vkToken(token));
        if (rejected != null) return rejected;
        return _tokenPersisted
            ? null
            : 'Не удалось сохранить вход в защищённом хранилище устройства — '
                  'он действует до закрытия приложения';
    }
  }

  /// «Выйти»: токен стирается, движок возвращается к ручным хешам.
  Future<String?> logout() async {
    _token = null;
    _tokenPersisted = true;
    _changed();
    String? note;
    try {
      await store.deleteToken();
    } on Object {
      note = 'Не удалось стереть вход из защищённого хранилища устройства';
    }
    final rejected = await _send(const UserInput.vkToken(''));
    return note ?? rejected;
  }

  // ---------- ядро ----------

  void _onController() {
    final active = _isActive(controller.state.session);
    if (active && !_sessionActive) _epoch++;
    _sessionActive = active;
    _supplyIfAsked();
    // Экран и так перестраивается по кадрам ядра; сообщаем только о том, что
    // зависит от нас: «токен не принят» появилось или ушло.
    final rejected = tokenRejected;
    if (rejected != _wasRejected) {
      _wasRejected = rejected;
      _changed();
    }
  }

  static bool _isActive(SessionState session) =>
      session is! SessionIdle && session is! SessionNeedsLink;

  /// Движок спросил хеши, а у нас есть сохранённое — отдаём один раз за сессию.
  void _supplyIfAsked() {
    final engine = this.engine;
    if (!_loaded || engine == null || !hasCredentials) return;
    final asking =
        engine.state == EngineStatus.waitingUser &&
        engine.userAction is NeedHashesAction;
    if (!asking || _suppliedEpoch == _epoch) return;
    _suppliedEpoch = _epoch;
    unawaited(_supplyStored());
  }

  Future<void> _supplyStored() async {
    // Токен первым: с ним ядро создаёт звонки само, хеши остаются запасными.
    final token = _token;
    if (token != null) await _send(UserInput.vkToken(token));
    if (_hashes.isNotEmpty) await _send(UserInput.vkHashes(_hashes));
  }

  /// Отправить движку. `null` — принято или отложено до первой сессии; иначе
  /// текст отказа.
  Future<String?> _send(UserInput input) async {
    final id = engine?.id;
    if (id == null) return null;
    try {
      await controller.userInput(id, input);
      return null;
    } on BridgeError catch (error) {
      return _isPending(error) ? null : error.message;
    } on Object catch (error) {
      return BraidController.describeError(error);
    }
  }

  static bool _isPending(BridgeError error) =>
      error.kind == BridgeErrorKind.notReady ||
      error.kind == BridgeErrorKind.notConnected ||
      error.message == _engineNotStarted;
}
