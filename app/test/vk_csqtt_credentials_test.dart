// Хеши звонков и вход через VK для CSQTT (ист. 14–17, 78): что хранится на
// устройстве, что уходит движку в `user_input` и когда. Ядро хранит хеши и
// токен только в памяти движка, поэтому после перезапуска приложения их
// возвращает экран — один раз за сессию, без зацикливания, когда ядро
// просит новые.

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/services/vk/csqtt_credentials.dart';
import 'package:braid/services/vk/vk_oauth.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/rig.dart';
import 'support/states.dart';

const hashA = 'kw6VzMMQPO8fEtP0Yoo6_8nbuKfTCjxNq1GJdpQTnUA';
const hashB = 'abcdefghijklmnopQRST';
const linkA = 'https://vk.com/call/join/$hashA';

/// Ядро, которое отвечает на `user_input` так, как ответило бы без сессии.
class StrictBridge extends FakeBridge {
  StrictBridge({super.state});

  BridgeError? userInputError;

  @override
  Future<void> userInput(String engine, UserInput input) async {
    final error = userInputError;
    if (error != null) throw error;
    return super.userInput(engine, input);
  }
}

const notStarted = BridgeError(
  kind: BridgeErrorKind.failed,
  message: 'движок не запущен',
  details: [],
);

CoreState waiting(UserAction action, {SessionState? session}) => idleState(
  session: session ?? const SessionConnected(),
  active: 'awg',
  engines: [
    const EngineView(id: 'awg', kind: 'awg', state: EngineStatus.connected),
    EngineView(
      id: 'csqtt',
      kind: 'csqtt',
      state: EngineStatus.waitingUser,
      userAction: action,
    ),
  ],
);

Future<(CsqttCredentials, Rig, MemoryVkStore)> setUp1({
  MemoryVkStore? store,
  StrictBridge? bridge,
  CoreState? state,
}) async {
  final rig = Rig(
    bridge: bridge ?? StrictBridge(state: state ?? idleState()),
    autoAnswer: false,
  );
  await rig.start();
  await Future<void>.delayed(Duration.zero);
  final memory = store ?? MemoryVkStore();
  final credentials = CsqttCredentials(
    controller: rig.controller,
    store: memory,
    oauth: VkOAuth(launcher: FakeWebLauncher()),
  );
  addTearDown(() async {
    credentials.dispose();
    await rig.dispose();
  });
  await credentials.load();
  return (credentials, rig, memory);
}

List<(String, UserInput)> sent(Rig rig) => rig.bridge.userInputs;

void main() {
  group('хеши', () {
    test('читаются из хранилища при запуске', () async {
      final (credentials, _, _) = await setUp1(
        store: MemoryVkStore(hashes: [hashA], token: 'vk-token'),
      );
      expect(credentials.loaded, isTrue);
      expect(credentials.hashes, [hashA]);
      expect(credentials.loggedIn, isTrue);
    });

    test('кривой ввод: ошибка на месте, список и ядро не тронуты', () async {
      final (credentials, rig, store) = await setUp1();
      final error = await credentials.addHash(
        'https://example.com/call/join/abc',
      );
      expect(error, 'Это не ссылка на звонок VK');
      expect(credentials.hashes, isEmpty);
      expect(store.hashes, isEmpty);
      expect(sent(rig), isEmpty);
    });

    test('ссылка нормализуется до хеша, сохраняется и уходит движку', () async {
      final (credentials, rig, store) = await setUp1();
      expect(await credentials.addHash('  $linkA  '), isNull);
      expect(credentials.hashes, [hashA]);
      expect(store.hashes, [hashA]);
      expect(sent(rig), [
        ('csqtt', const UserInput.vkHashes([hashA])),
      ]);
    });

    test('тот же звонок второй раз не добавляется', () async {
      final (credentials, rig, _) = await setUp1();
      await credentials.addHash(linkA);
      final error = await credentials.addHash(hashA);
      expect(error, 'Этот звонок уже в списке');
      expect(credentials.hashes, [hashA]);
      expect(sent(rig), hasLength(1));
    });

    test(
      'сессии ещё нет: хеш сохранён, ошибки нет, отправится позже',
      () async {
        final bridge = StrictBridge()..userInputError = notStarted;
        final (credentials, rig, store) = await setUp1(bridge: bridge);
        expect(await credentials.addHash(linkA), isNull);
        expect(store.hashes, [hashA]);
        expect(credentials.hashes, [hashA]);

        // Подключились; движок спросил хеши — экран отдаёт сохранённые.
        bridge.userInputError = null;
        rig.bridge.emit(next: waiting(const NeedHashesAction()));
        await Future<void>.delayed(Duration.zero);
        expect(sent(rig), [
          ('csqtt', const UserInput.vkHashes([hashA])),
        ]);
      },
    );

    test('ядро отвергло ввод: список откатывается, текст — из ядра', () async {
      final bridge = StrictBridge()
        ..userInputError = const BridgeError(
          kind: BridgeErrorKind.failed,
          message: 'Это не ссылка на звонок VK',
          details: [],
        );
      final (credentials, _, store) = await setUp1(bridge: bridge);
      expect(await credentials.addHash(linkA), 'Это не ссылка на звонок VK');
      expect(credentials.hashes, isEmpty);
      expect(store.hashes, isEmpty);
    });

    test('удаление хеша обновляет хранилище и движок', () async {
      final (credentials, rig, store) = await setUp1(
        store: MemoryVkStore(hashes: [hashA, hashB]),
      );
      await credentials.removeHash(hashA);
      expect(credentials.hashes, [hashB]);
      expect(store.hashes, [hashB]);
      expect(sent(rig).last, ('csqtt', const UserInput.vkHashes([hashB])));
    });
  });

  group('возврат сохранённого после перезапуска', () {
    test(
      'движок просит хеши — токен и хеши уходят один раз за сессию',
      () async {
        final (credentials, rig, _) = await setUp1(
          store: MemoryVkStore(hashes: [hashA], token: 'vk-token'),
        );
        rig.bridge.emit(next: waiting(const NeedHashesAction()));
        await Future<void>.delayed(Duration.zero);
        expect(sent(rig), [
          ('csqtt', const UserInput.vkToken('vk-token')),
          ('csqtt', const UserInput.vkHashes([hashA])),
        ]);

        // Ядро просит снова (хеши исчерпаны) — повторно не отдаём: цикла нет.
        rig.bridge.emit(next: idleState(session: const SessionConnected()));
        rig.bridge.emit(next: waiting(const NeedHashesAction()));
        await Future<void>.delayed(Duration.zero);
        expect(sent(rig), hasLength(2));
        expect(credentials.hashes, [hashA], reason: 'сохранённое не стёрто');
      },
    );

    test('в новой сессии сохранённое отдаётся снова', () async {
      final (_, rig, _) = await setUp1(store: MemoryVkStore(hashes: [hashA]));
      rig.bridge.emit(next: waiting(const NeedHashesAction()));
      await Future<void>.delayed(Duration.zero);
      expect(sent(rig), hasLength(1));

      rig.bridge.emit(next: idleState());
      rig.bridge.emit(
        next: waiting(
          const NeedHashesAction(),
          session: const SessionConnecting(),
        ),
      );
      await Future<void>.delayed(Duration.zero);
      expect(sent(rig), hasLength(2));
    });

    test('сохранённого нет — движку ничего не отправляется', () async {
      final (_, rig, _) = await setUp1();
      rig.bridge.emit(next: waiting(const NeedHashesAction()));
      await Future<void>.delayed(Duration.zero);
      expect(sent(rig), isEmpty);
    });

    test(
      'токен не принят: вход нужен заново, токен второй раз не шлём',
      () async {
        final (credentials, rig, _) = await setUp1(
          store: MemoryVkStore(token: 'old-token'),
        );
        rig.bridge.emit(next: waiting(const NeedHashesAction()));
        await Future<void>.delayed(Duration.zero);
        expect(sent(rig), [('csqtt', const UserInput.vkToken('old-token'))]);

        rig.bridge.emit(next: waiting(const VkLoginAction()));
        await Future<void>.delayed(Duration.zero);
        expect(sent(rig), hasLength(1));
        expect(credentials.tokenRejected, isTrue);
      },
    );
  });

  group('вход через VK', () {
    test('токен сохраняется, уходит движку, статус «вошли»', () async {
      final (credentials, rig, store) = await setUp1();
      final note = await credentials.completeLogin(
        const VkLoginSucceeded('vk-token'),
      );
      expect(note, isNull);
      expect(credentials.loggedIn, isTrue);
      expect(store.token, 'vk-token');
      expect(sent(rig), [('csqtt', const UserInput.vkToken('vk-token'))]);
    });

    test('вход не удался: текст отказа, ничего не сохранено', () async {
      final (credentials, rig, store) = await setUp1();
      final note = await credentials.completeLogin(
        const VkLoginFailed('Окно закрыто до завершения входа'),
      );
      expect(note, 'Окно закрыто до завершения входа');
      expect(credentials.loggedIn, isFalse);
      expect(store.token, isNull);
      expect(sent(rig), isEmpty);
    });

    test(
      'хранилище устройства не приняло токен: вход действует до закрытия',
      () async {
        final store = MemoryVkStore()..writeError = StateError('keychain');
        final (credentials, rig, _) = await setUp1(store: store);
        final note = await credentials.completeLogin(
          const VkLoginSucceeded('vk-token'),
        );
        expect(note, contains('защищённом хранилище'));
        expect(credentials.loggedIn, isTrue);
        expect(credentials.tokenPersisted, isFalse);
        expect(sent(rig), [('csqtt', const UserInput.vkToken('vk-token'))]);
      },
    );

    test(
      '«Выйти» стирает токен, пустой токен уходит движку, хеши остаются',
      () async {
        final (credentials, rig, store) = await setUp1(
          store: MemoryVkStore(hashes: [hashA], token: 'vk-token'),
        );
        await credentials.logout();
        expect(credentials.loggedIn, isFalse);
        expect(store.token, isNull);
        expect(store.hashes, [hashA]);
        expect(sent(rig), [('csqtt', const UserInput.vkToken(''))]);
      },
    );
  });

  test('CSQTT в подписке нет — всё сохраняется, движку слать некому', () async {
    final (credentials, rig, store) = await setUp1(
      state: idleState(engines: const [awgEngine]),
    );
    expect(credentials.engine, isNull);
    expect(await credentials.addHash(hashB), isNull);
    expect(store.hashes, [hashB]);
    expect(sent(rig), isEmpty);
  });
}
