// Экран «Подписка» (ист. 35, 36, 37, 38, 47, 76): хост, движки, наборы, дата,
// «Обновить сейчас», ошибка последней попытки, «не обновлялась с…», «Доступ
// отозван»; QR и копирование; замена ссылкой с подтверждением.

import 'dart:async';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/screens/subscription/link_qr.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

final subscribed = SubscriptionView(
  hostId: 'frankfurt',
  hostName: 'Франкфурт',
  version: 7,
  updatedAt: DateTime.utc(2026, 10, 15, 12),
);

Future<Rig> openSubscription(
  WidgetTester tester, {
  CoreState? state,
  FakeBridge? bridge,
  FakeClipboard? clipboard,
  Rig? rig,
}) async {
  final app =
      rig ??
      Rig(
        bridge: (bridge ?? FakeBridge())
          ..state = state ?? idleState(subscription: subscribed),
        clipboard: clipboard,
        autoAnswer: false,
      );
  await pumpApp(tester, rig: app);
  await openTile(tester, 'tile-subscription');
  return app;
}

String fieldText(WidgetTester tester) => tester
    .widget<TextField>(find.byKey(const Key('new-link-field')))
    .controller!
    .text;

void main() {
  testWidgets('хост, версия, дата обновления, движки и наборы', (tester) async {
    await openSubscription(tester);

    expect(find.byKey(const Key('sub-host')), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(const Key('sub-host')),
        matching: find.text('Франкфурт'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: find.byKey(const Key('sub-version')),
        matching: find.text('7'),
      ),
      findsOneWidget,
    );
    final updated = find.byKey(const Key('sub-updated'));
    expect(
      find.descendant(of: updated, matching: find.textContaining('окт')),
      findsOneWidget,
    );

    final engines = tester
        .widget<Text>(find.byKey(const Key('sub-engines')))
        .data!;
    expect(engines, contains('AWG'));
    expect(engines, contains('CSQTT'));
    final sets = tester.widget<Text>(find.byKey(const Key('sub-sets'))).data!;
    expect(sets, contains('Google'));
    expect(sets, contains('Cursor'));
  });

  testWidgets('подписка ни разу не обновлялась — так и сказано', (
    tester,
  ) async {
    await openSubscription(
      tester,
      state: idleState(
        subscription: const SubscriptionView(
          hostId: 'frankfurt',
          hostName: 'Франкфурт',
          version: 1,
        ),
      ),
    );
    expect(
      find.descendant(
        of: find.byKey(const Key('sub-updated')),
        matching: find.text('ещё не обновлялась'),
      ),
      findsOneWidget,
    );
  });

  testWidgets('ошибка последней попытки и «не обновлялась с …» видны', (
    tester,
  ) async {
    await openSubscription(
      tester,
      state: idleState(
        subscription: SubscriptionView(
          hostId: 'frankfurt',
          hostName: 'Франкфурт',
          version: 7,
          updatedAt: DateTime.utc(2026, 10, 15, 12),
          lastRefreshError: 'Хост недоступен',
        ),
        banners: [
          SubscriptionStaleBanner(since: DateTime.utc(2026, 10, 15, 12)),
        ],
      ),
    );
    expect(find.byKey(const Key('sub-error')), findsOneWidget);
    expect(find.textContaining('Хост недоступен'), findsOneWidget);
    expect(find.byKey(const Key('sub-stale')), findsOneWidget);
    expect(find.textContaining('Не обновлялась с'), findsOneWidget);
  });

  testWidgets('«Доступ отозван» — отдельная плашка с подсказкой', (
    tester,
  ) async {
    await openSubscription(
      tester,
      state: idleState(
        subscription: const SubscriptionView(
          hostId: 'frankfurt',
          hostName: 'Франкфурт',
          version: 7,
          revoked: true,
        ),
        banners: const [RevokedBanner()],
      ),
    );
    expect(find.byKey(const Key('sub-revoked')), findsOneWidget);
    expect(find.text('Доступ отозван'), findsOneWidget);
    expect(find.textContaining('новую ссылку'), findsWidgets);
  });

  testWidgets('«Обновить сейчас» обновляет и сообщает итог', (tester) async {
    final bridge = FakeBridge()..refreshOutcome = const Updated(8);
    final rig = await openSubscription(tester, bridge: bridge);

    await tester.tap(find.byKey(const Key('refresh-now')));
    await tester.pumpAndSettle();

    expect(rig.bridge.calls, contains('refreshSubscription'));
    expect(find.text('Подписка обновлена · версия 8'), findsOneWidget);
  });

  testWidgets('пока идёт обновление, кнопка занята', (tester) async {
    final bridge = FakeBridge()..refreshGate = Completer<void>();
    await openSubscription(tester, bridge: bridge);

    await tester.tap(find.byKey(const Key('refresh-now')));
    await tester.pump();
    expect(
      tester
          .widget<FilledButton>(find.byKey(const Key('refresh-now')))
          .onPressed,
      isNull,
    );
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    expect(bridge.calls.where((c) => c == 'refreshSubscription'), hasLength(1));

    bridge.refreshGate!.complete();
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<FilledButton>(find.byKey(const Key('refresh-now')))
          .onPressed,
      isNotNull,
    );
  });

  testWidgets('ошибка обновления — текстом ядра', (tester) async {
    final bridge = FakeBridge()
      ..refreshError = const BridgeError(
        kind: BridgeErrorKind.failed,
        message: 'Хост недоступен',
        details: [],
      );
    await openSubscription(tester, bridge: bridge);
    await tester.tap(find.byKey(const Key('refresh-now')));
    await tester.pumpAndSettle();
    expect(find.text('Хост недоступен'), findsOneWidget);
  });

  testWidgets('QR текущей ссылки показывается и копируется (десктоп)', (
    tester,
  ) async {
    final clipboard = FakeClipboard();
    await openSubscription(tester, clipboard: clipboard);

    await reveal(tester, find.byKey(const Key('link-show-qr')));
    await tester.tap(find.byKey(const Key('link-show-qr')));
    await tester.pumpAndSettle();
    expect(find.byType(LinkQr), findsOneWidget);
    expect(
      tester.widget<LinkQr>(find.byType(LinkQr)).data,
      'braid://current-subscription',
    );

    await tester.tap(find.byKey(const Key('link-copy-dialog')));
    await tester.pumpAndSettle();
    expect(clipboard.text, 'braid://current-subscription');
    expect(find.text('Ссылка скопирована'), findsOneWidget);
  });

  testWidgets('ссылку можно скопировать, не открывая QR', (tester) async {
    final clipboard = FakeClipboard();
    await openSubscription(tester, clipboard: clipboard);
    await reveal(tester, find.byKey(const Key('link-copy')));
    await tester.tap(find.byKey(const Key('link-copy')));
    await tester.pumpAndSettle();
    expect(clipboard.text, 'braid://current-subscription');
    expect(find.byType(LinkQr), findsNothing);
  });

  group('смена ссылки', () {
    testWidgets(
      'ссылка другого хоста спрашивает замену; согласие меняет подписку',
      (tester) async {
        final rig = await openSubscription(tester);
        await reveal(tester, find.byKey(const Key('new-link-field')));

        await tester.enterText(
          find.byKey(const Key('new-link-field')),
          'braid://other',
        );
        await tester.tap(find.byKey(const Key('new-link-button')));
        await tester.pumpAndSettle();

        expect(find.text('Заменить подписку?'), findsOneWidget);
        expect(find.textContaining('Франкфурт'), findsWidgets);
        expect(find.textContaining('new.example'), findsOneWidget);
        expect(
          rig.bridge.replacedLinks,
          isEmpty,
          reason: 'без подтверждения ничего не меняется',
        );

        await tester.tap(find.byKey(const Key('confirm-replace-yes')));
        await tester.pumpAndSettle();
        expect(rig.bridge.replacedLinks, ['braid://other']);
        expect(
          find.descendant(
            of: find.byKey(const Key('sub-host')),
            matching: find.text('Хельсинки'),
          ),
          findsOneWidget,
        );
        expect(fieldText(tester), isEmpty);
      },
    );

    testWidgets(
      'отказ от замены оставляет прежнюю подписку, ввод не теряется',
      (tester) async {
        final rig = await openSubscription(tester);
        await reveal(tester, find.byKey(const Key('new-link-field')));
        await tester.enterText(
          find.byKey(const Key('new-link-field')),
          'braid://other',
        );
        await tester.tap(find.byKey(const Key('new-link-button')));
        await tester.pumpAndSettle();

        await tester.tap(find.text('Отмена'));
        await tester.pumpAndSettle();
        expect(rig.bridge.replacedLinks, isEmpty);
        expect(
          find.descendant(
            of: find.byKey(const Key('sub-host')),
            matching: find.text('Франкфурт'),
          ),
          findsOneWidget,
        );
        expect(fieldText(tester), 'braid://other');
      },
    );

    testWidgets('не ссылка Braid — ошибка на месте, ввод остаётся', (
      tester,
    ) async {
      await openSubscription(tester);
      await reveal(tester, find.byKey(const Key('new-link-field')));
      await tester.enterText(find.byKey(const Key('new-link-field')), 'привет');
      await tester.tap(find.byKey(const Key('new-link-button')));
      await tester.pumpAndSettle();
      expect(find.text('Это не ссылка Braid'), findsOneWidget);
      expect(fieldText(tester), 'привет');
    });

    testWidgets(
      'доступ отозван: новая ссылка вставляется сюда же и заменяет подписку',
      (tester) async {
        final revoked = idleState(
          subscription: const SubscriptionView(
            hostId: 'frankfurt',
            hostName: 'Франкфурт',
            version: 7,
            revoked: true,
          ),
          banners: const [RevokedBanner()],
        );
        final rig = await openSubscription(tester, state: revoked);
        await reveal(tester, find.byKey(const Key('new-link-field')));
        await tester.enterText(
          find.byKey(const Key('new-link-field')),
          'braid://fresh',
        );
        await tester.tap(find.byKey(const Key('new-link-button')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(const Key('confirm-replace-yes')));
        await tester.pumpAndSettle();
        expect(rig.bridge.replacedLinks, ['braid://fresh']);
      },
    );

    testWidgets('Android: сканер QR заполняет поле и заменяет подписку', (
      tester,
    ) async {
      final scanner = FakeQrScanner('braid://from-qr');
      final rig = Rig(
        bridge: FakeBridge(
          state: idleState(subscription: subscribed),
          caps: androidCaps,
        ),
        platform: android,
        vpn: FakeVpn(),
        scanner: scanner,
        autoAnswer: false,
      );
      await openSubscription(tester, rig: rig);
      await reveal(tester, find.byKey(const Key('new-link-scan')));
      await tester.tap(find.byKey(const Key('new-link-scan')));
      await tester.pumpAndSettle();
      expect(scanner.opened, 1);
      expect(find.text('Заменить подписку?'), findsOneWidget);
    });

    testWidgets('на десктопе сканера в этом экране нет', (tester) async {
      await openSubscription(tester);
      expect(find.byKey(const Key('new-link-scan')), findsNothing);
    });
  });

  testWidgets('кнопка на плашке «Новая ссылка» ведёт на этот экран', (
    tester,
  ) async {
    final revoked = idleState(
      subscription: const SubscriptionView(
        hostId: 'frankfurt',
        hostName: 'Франкфурт',
        version: 7,
        revoked: true,
      ),
      banners: const [RevokedBanner()],
    );
    await pumpApp(
      tester,
      rig: Rig(bridge: FakeBridge(state: revoked), autoAnswer: false),
    );
    await tester.tap(find.byKey(const Key('banner-revoked-new-link')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('sub-revoked')), findsOneWidget);
    expect(find.byKey(const Key('sub-host')), findsOneWidget);
  });
}
