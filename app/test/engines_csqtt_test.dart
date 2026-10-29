// CSQTT на экране «Движки» (ист. 14–17, 78): список хешей звонков с проверкой,
// ошибка «Это не ссылка на звонок VK» на месте, «Войти через VK» через WebView
// с токеном в защищённом хранилище, «Нужны новые хеши» с подсказкой.

import 'package:braid/app_extensions.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/engines/engines_screen.dart';
import 'package:braid/services/vk/vk_oauth.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

const hashA = 'kw6VzMMQPO8fEtP0Yoo6_8nbuKfTCjxNq1GJdpQTnUA';
const hashB = 'abcdefghijklmnopQRST';
const linkA = 'https://vk.com/call/join/$hashA';
const redirect =
    'https://oauth.vk.ru/blank.html#access_token=synthetic-token&expires_in=0';

class Ctx {
  Ctx(this.rig, this.store, this.web);

  final Rig rig;
  final MemoryVkStore store;
  final FakeWebLauncher web;

  FakeWebSession get page => web.session;
}

Future<Ctx> openCsqtt(
  WidgetTester tester, {
  CoreState? state,
  MemoryVkStore? store,
}) async {
  final launcher = FakeWebLauncher();
  final memory = store ?? MemoryVkStore();
  final rig = Rig(
    bridge: FakeBridge()..state = state ?? connectedState(),
    autoAnswer: false,
  );
  EngineServices.install(
    EngineServices(
      controller: rig.controller,
      store: memory,
      web: launcher,
      oauth: VkOAuth(
        launcher: launcher,
        interval: const Duration(milliseconds: 5),
      ),
      statsFiles: FakeStatsFiles(),
    ),
  );
  await pumpApp(
    tester,
    rig: rig,
    extensions: const AppExtensions(engines: buildEnginesScreen),
  );
  await openTile(tester, 'tile-engines');
  return Ctx(rig, memory, launcher);
}

Future<void> typeHash(WidgetTester tester, String text) async {
  await reveal(tester, find.byKey(const Key('hash-input')));
  await tester.enterText(find.byKey(const Key('hash-input')), text);
  await tester.pump();
}

Future<void> addHash(WidgetTester tester) async {
  await reveal(tester, find.byKey(const Key('hash-add')));
  await tester.tap(find.byKey(const Key('hash-add')));
  await tester.pumpAndSettle();
}

String inputText(WidgetTester tester) => tester
    .widget<TextField>(find.byKey(const Key('hash-input')))
    .controller!
    .text;

CoreState asking(UserAction action) => idleState(
  session: const SessionConnected(),
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

/// Открыть вход через VK и довести до результата страницей.
Future<void> loginWith(
  WidgetTester tester,
  Ctx ctx, {
  String? returnUrl,
}) async {
  await reveal(tester, find.byKey(const Key('vk-login')));
  await tester.tap(find.byKey(const Key('vk-login')));
  await tester.pump();
  await tester.pump();
  if (returnUrl != null) {
    ctx.page.url = returnUrl;
  } else {
    ctx.page.userCloses();
  }
  await tester.pump(const Duration(milliseconds: 100));
  await tester.pumpAndSettle();
}

void main() {
  group('хеши звонков', () {
    testWidgets('раздел есть, только если в подписке есть CSQTT', (
      tester,
    ) async {
      await openCsqtt(tester, state: idleState(engines: const [awgEngine]));
      expect(find.byKey(const Key('csqtt-section')), findsNothing);
    });

    testWidgets('кривой ввод: ошибка на месте, введённое не теряется', (
      tester,
    ) async {
      final ctx = await openCsqtt(tester);
      await typeHash(tester, 'https://example.com/call/join/abc');
      await addHash(tester);

      expect(find.text('Это не ссылка на звонок VK'), findsOneWidget);
      expect(inputText(tester), 'https://example.com/call/join/abc');
      expect(find.byKey(const Key('hash-0')), findsNothing);
      expect(ctx.rig.bridge.userInputs, isEmpty);
      expect(ctx.store.hashes, isEmpty);
    });

    testWidgets('пустой ввод: просим вставить ссылку, ядру ничего не шлём', (
      tester,
    ) async {
      final ctx = await openCsqtt(tester);
      await typeHash(tester, '   ');
      await addHash(tester);
      expect(find.text('Вставь ссылку на звонок VK или хеш'), findsOneWidget);
      expect(ctx.rig.bridge.userInputs, isEmpty);
    });

    testWidgets('ошибка уходит, когда начали править ввод', (tester) async {
      await openCsqtt(tester);
      await typeHash(tester, 'мусор');
      await addHash(tester);
      expect(find.text('Это не ссылка на звонок VK'), findsOneWidget);

      await typeHash(tester, 'мусор2');
      expect(find.text('Это не ссылка на звонок VK'), findsNothing);
    });

    testWidgets('ссылка добавляется: хеш в списке укороченным, ввод пуст', (
      tester,
    ) async {
      final ctx = await openCsqtt(tester);
      await typeHash(tester, linkA);
      await addHash(tester);

      expect(find.text('kw6VzM…QTnUA'), findsOneWidget);
      expect(find.text(hashA), findsNothing, reason: 'хеш целиком не светим');
      expect(inputText(tester), isEmpty);
      expect(find.text('Это не ссылка на звонок VK'), findsNothing);
      expect(ctx.store.hashes, [hashA]);
      expect(ctx.rig.bridge.userInputs, [
        ('csqtt', const UserInput.vkHashes([hashA])),
      ]);
    });

    testWidgets('сохранённые хеши видны при открытии экрана', (tester) async {
      await openCsqtt(tester, store: MemoryVkStore(hashes: [hashA, hashB]));
      await reveal(tester, find.byKey(const Key('hash-1')));
      expect(find.text('kw6VzM…QTnUA'), findsOneWidget);
      expect(find.text('abcdef…pQRST'), findsOneWidget);
    });

    testWidgets('хеш удаляется из списка, хранилища и движка', (tester) async {
      final ctx = await openCsqtt(
        tester,
        store: MemoryVkStore(hashes: [hashA, hashB]),
      );
      await reveal(tester, find.byKey(const Key('hash-remove-0')));
      await tester.tap(find.byKey(const Key('hash-remove-0')));
      await tester.pumpAndSettle();

      expect(find.byKey(const Key('hash-1')), findsNothing);
      expect(find.text('abcdef…pQRST'), findsOneWidget);
      expect(ctx.store.hashes, [hashB]);
      expect(ctx.rig.bridge.userInputs.last, (
        'csqtt',
        const UserInput.vkHashes([hashB]),
      ));
    });

    testWidgets('статус каждого хеша: формат проверен', (tester) async {
      await openCsqtt(
        tester,
        state: idleState(),
        store: MemoryVkStore(hashes: [hashA]),
      );
      await reveal(tester, find.byKey(const Key('hash-0')));
      expect(
        find.descendant(
          of: find.byKey(const Key('hash-0')),
          matching: find.text('Формат проверен'),
        ),
        findsOneWidget,
      );
    });

    testWidgets(
      'хеши исчерпаны: «Нужны новые хеши», подсказка и статус каждого',
      (tester) async {
        await openCsqtt(
          tester,
          state: asking(const NeedHashesAction()),
          store: MemoryVkStore(hashes: [hashA]),
        );
        await reveal(tester, find.byKey(const Key('csqtt-hint')));
        expect(find.text('Нужны новые хеши'), findsWidgets);
        expect(
          find.descendant(
            of: find.byKey(const Key('csqtt-hint')),
            matching: find.textContaining('https://vk.com/call/join/'),
          ),
          findsOneWidget,
          reason: 'подсказка, как получить ссылку',
        );
        expect(
          find.descendant(
            of: find.byKey(const Key('hash-0')),
            matching: find.text('Звонок недоступен'),
          ),
          findsOneWidget,
        );
      },
    );

    testWidgets('подсказки про новые хеши нет, пока хеши работают', (
      tester,
    ) async {
      await openCsqtt(tester, store: MemoryVkStore(hashes: [hashA]));
      expect(find.byKey(const Key('csqtt-hint')), findsNothing);
    });
  });

  group('вход через VK', () {
    testWidgets('«Войти через VK» открывает окно входа и диалог', (
      tester,
    ) async {
      final ctx = await openCsqtt(tester);
      expect(find.text('VK: не вошли'), findsOneWidget);

      await reveal(tester, find.byKey(const Key('vk-login')));
      await tester.tap(find.byKey(const Key('vk-login')));
      await tester.pump();
      await tester.pump();

      expect(find.byKey(const Key('vk-login-dialog')), findsOneWidget);
      expect(find.text('Вход через VK'), findsWidgets);
      expect(ctx.web.requests.single.url, const VkOAuthConfig().authorizeUrl);

      ctx.page.userCloses();
      await tester.pump(const Duration(milliseconds: 100));
      await tester.pumpAndSettle();
    });

    testWidgets(
      'вошли: «VK: вошли», токен сохранён и отдан движку, хеши прячутся',
      (tester) async {
        final ctx = await openCsqtt(
          tester,
          store: MemoryVkStore(hashes: [hashA]),
        );
        await loginWith(tester, ctx, returnUrl: redirect);

        expect(find.byKey(const Key('vk-login-dialog')), findsNothing);
        expect(find.text('VK: вошли'), findsOneWidget);
        expect(find.byKey(const Key('vk-logout')), findsOneWidget);
        expect(find.byKey(const Key('vk-login')), findsNothing);
        expect(find.byKey(const Key('hash-input')), findsNothing);
        expect(
          find.text('Хеши вводить не нужно — звонки создаются сами'),
          findsOneWidget,
        );
        expect(ctx.store.token, 'synthetic-token');
        expect(ctx.rig.bridge.userInputs, [
          ('csqtt', const UserInput.vkToken('synthetic-token')),
        ]);
      },
    );

    testWidgets('окно закрыли до конца входа — причина на месте, не вошли', (
      tester,
    ) async {
      final ctx = await openCsqtt(tester);
      await loginWith(tester, ctx);

      expect(find.byKey(const Key('vk-login-dialog')), findsNothing);
      expect(find.text('Окно закрыто до завершения входа'), findsOneWidget);
      expect(find.text('VK: не вошли'), findsOneWidget);
      expect(ctx.store.token, isNull);
      expect(ctx.rig.bridge.userInputs, isEmpty);
    });

    testWidgets('«Выйти» стирает токен и возвращает ручные хеши', (
      tester,
    ) async {
      final ctx = await openCsqtt(
        tester,
        store: MemoryVkStore(token: 'vk-token', hashes: [hashA]),
      );
      expect(find.text('VK: вошли'), findsOneWidget);

      await reveal(tester, find.byKey(const Key('vk-logout')));
      await tester.tap(find.byKey(const Key('vk-logout')));
      await tester.pumpAndSettle();

      expect(find.text('VK: не вошли'), findsOneWidget);
      expect(find.byKey(const Key('hash-input')), findsOneWidget);
      expect(find.byKey(const Key('hash-0')), findsOneWidget);
      expect(ctx.store.token, isNull);
      expect(ctx.rig.bridge.userInputs.last, (
        'csqtt',
        const UserInput.vkToken(''),
      ));
    });

    testWidgets('токен не принят: просим войти заново', (tester) async {
      await openCsqtt(
        tester,
        state: asking(const VkLoginAction()),
        store: MemoryVkStore(token: 'old-token'),
      );
      await reveal(tester, find.byKey(const Key('vk-status')));
      expect(find.text('VK: токен не принят — войди заново'), findsOneWidget);
      expect(find.byKey(const Key('vk-login')), findsOneWidget);
    });

    testWidgets('хранилище не приняло токен — вошли до закрытия приложения', (
      tester,
    ) async {
      final store = MemoryVkStore()..writeError = StateError('keychain');
      final ctx = await openCsqtt(tester, store: store);
      await loginWith(tester, ctx, returnUrl: redirect);

      expect(find.text('VK: вошли'), findsOneWidget);
      expect(find.textContaining('до закрытия приложения'), findsOneWidget);
    });
  });
}
