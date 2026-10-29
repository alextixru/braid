// Главный экран (ист. 1, 6, 9, 30, 31, 54, 58, 73): кнопка и состояние в
// четырёх видах (подключаюсь / подключено / нет связи / VPN перехвачен),
// сеть, режим-пресет, переключатель движка, «+ Домен» и баннеры.

import 'package:braid/app_extensions.dart';
import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/settings.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

final connectButton = find.byKey(const Key('connect-button'));

Finder inConnect(String text) =>
    find.descendant(of: connectButton, matching: find.text(text));

Rig rigWith(CoreState state, {FakeClipboard? clipboard, FakeBridge? bridge}) {
  return Rig(
    bridge: (bridge ?? FakeBridge())..state = state,
    clipboard: clipboard,
    autoAnswer: false,
  );
}

/// Диалог поверх экрана, где крутится кольцо «Подключаюсь…»: тишины не будет,
/// ждём появления диалога по времени.
Future<void> openDialog(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 400));
}

const rule = PrivilegeStatus(
  supported: true,
  ruleInstalled: false,
  helperCurrent: false,
);

void main() {
  group('кнопка и состояние', () {
    testWidgets('отключено: «Подключить» запускает подключение', (
      tester,
    ) async {
      final rig = await pumpApp(tester, rig: rigWith(idleState()));

      expect(find.text('Отключено'), findsOneWidget);
      expect(inConnect('Подключить'), findsOneWidget);

      await tester.tap(connectButton);
      await tester.pumpAndSettle();
      expect(rig.bridge.calls, contains('connect'));
      expect(find.text('Подключено · AWG · только что'), findsOneWidget);
      expect(inConnect('Отключить'), findsOneWidget);
    });

    testWidgets('подключаюсь: «Подключаюсь…», та же кнопка отменяет', (
      tester,
    ) async {
      final rig = await pumpApp(
        tester,
        rig: rigWith(idleState(session: const SessionConnecting())),
        settle: false,
      );
      await tester.pump(const Duration(milliseconds: 100));

      expect(find.text('Подключаюсь…'), findsOneWidget);
      expect(inConnect('Отменить'), findsOneWidget);

      await tester.tap(connectButton);
      await tester.pump(const Duration(milliseconds: 100));
      expect(rig.bridge.calls, contains('disconnect'));
    });

    testWidgets('подключено: «Подключено · AWG · 12 мин», время идёт', (
      tester,
    ) async {
      final rig = await pumpApp(tester, rig: rigWith(idleState()));

      rig.bridge.emit(next: idleState(session: const SessionConnecting()));
      await tester.pump();
      rig.bridge.emit(next: connectedState());
      await tester.pump();
      expect(find.text('Подключено · AWG · только что'), findsOneWidget);

      rig.clock = rig.clock.add(const Duration(minutes: 12));
      await tester.pump(const Duration(seconds: 30));
      expect(find.text('Подключено · AWG · 12 мин'), findsOneWidget);
      expect(inConnect('Отключить'), findsOneWidget);

      await tester.tap(connectButton);
      await tester.pumpAndSettle();
      expect(rig.bridge.calls.last, 'disconnect');
      expect(find.text('Отключено'), findsOneWidget);
    });

    testWidgets('нет связи с хостом: причина и «Повторить»', (tester) async {
      final rig = await pumpApp(
        tester,
        rig: rigWith(
          idleState(session: const SessionNoHost('Проба к хосту не ответила')),
        ),
      );

      expect(find.text('Нет связи с хостом'), findsOneWidget);
      expect(find.text('Проба к хосту не ответила'), findsOneWidget);
      expect(inConnect('Повторить'), findsOneWidget);
      expect(find.byKey(const Key('disconnect-button')), findsOneWidget);

      await tester.tap(connectButton);
      await tester.pumpAndSettle();
      expect(rig.bridge.calls, contains('connect'));
    });

    testWidgets('нет связи: «Отключить» рядом снимает защиту совсем', (
      tester,
    ) async {
      final rig = await pumpApp(
        tester,
        rig: rigWith(idleState(session: const SessionNoHost('нет ответа'))),
      );
      await tester.tap(find.byKey(const Key('disconnect-button')));
      await tester.pumpAndSettle();
      expect(rig.bridge.calls, contains('disconnect'));
      expect(rig.bridge.calls, isNot(contains('connect')));
    });

    testWidgets(
      'закреплённый движок мёртв: причина называет его по-человечески',
      (tester) async {
        await pumpApp(
          tester,
          rig: rigWith(
            idleState(
              session: const SessionNoHost('awg недоступен, закреплён вручную'),
            ),
          ),
        );
        expect(find.text('AWG недоступен, закреплён вручную'), findsOneWidget);
      },
    );

    testWidgets(
      'VPN перехвачен другим приложением: сообщение и кнопка вернуть',
      (tester) async {
        final rig = await pumpApp(
          tester,
          rig: rigWith(
            idleState(session: const SessionInterrupted(byOtherVpn: true)),
          ),
        );
        expect(find.text('VPN перехвачен другим приложением'), findsOneWidget);
        expect(inConnect('Вернуть VPN'), findsOneWidget);

        await tester.tap(connectButton);
        await tester.pumpAndSettle();
        expect(rig.bridge.calls, contains('connect'));
      },
    );

    testWidgets(
      'разрешение отозвано системой — тоже с кнопкой, без обвинения другого VPN',
      (tester) async {
        await pumpApp(
          tester,
          rig: rigWith(
            idleState(session: const SessionInterrupted(byOtherVpn: false)),
          ),
        );
        expect(find.text('Подключение прервано системой'), findsOneWidget);
        expect(find.text('VPN перехвачен другим приложением'), findsNothing);
        expect(inConnect('Вернуть VPN'), findsOneWidget);
      },
    );

    testWidgets('другой VPN держит маршруты: «Отключи <имя>» вместо плашки', (
      tester,
    ) async {
      await pumpApp(
        tester,
        rig: rigWith(
          idleState(banners: const [OtherVpnBanner(iface: 'utun3')]),
        ),
      );
      expect(find.text('Отключи utun3'), findsOneWidget);
      expect(inConnect('Подключить'), findsOneWidget);
      expect(find.byKey(const Key('banner-other-vpn')), findsNothing);
    });

    testWidgets('все состояния помещаются на узком экране 320×568', (
      tester,
    ) async {
      final states = <CoreState>[
        idleState(),
        idleState(session: const SessionConnecting()),
        connectedState(network: 'wifi:00aa11bb22cc33dd', admin: true),
        idleState(
          session: const SessionNoHost(
            'Проба к хосту не ответила, хендшейк старше 180 с',
          ),
        ),
        idleState(session: const SessionInterrupted(byOtherVpn: true)),
        idleState(banners: const [OtherVpnBanner(iface: 'utun3')]),
      ];
      for (final state in states) {
        await pumpApp(
          tester,
          rig: rigWith(state),
          size: const Size(320, 568),
          settle: false,
        );
        await tester.pump(const Duration(milliseconds: 300));
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox.shrink());
      }
    });
  });

  testWidgets(
    'кнопка подключения доступна читалке экрана: подпись, роль и действие',
    (tester) async {
      final handle = tester.ensureSemantics();
      await pumpApp(tester, rig: rigWith(idleState()));

      expect(
        tester.getSemantics(connectButton),
        matchesSemantics(
          label: 'Подключить',
          isButton: true,
          hasEnabledState: true,
          isEnabled: true,
          hasTapAction: true,
        ),
      );
      // Ручка семантики снимается в теле теста: фреймворк проверяет это до teardown.
      handle.dispose();
    },
  );

  group('сеть, режим, движок', () {
    testWidgets('сеть и режим-пресет видны; без режима — «Своё»', (
      tester,
    ) async {
      final rig = await pumpApp(
        tester,
        rig: rigWith(idleState(network: 'wifi:00aa11bb22cc33dd')),
      );
      expect(find.text('Wi‑Fi · 33dd'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const Key('mode-row')),
          matching: find.text('Работа'),
        ),
        findsOneWidget,
      );

      rig.bridge.emit(
        next: idleState(
          network: 'wifi:00aa11bb22cc33dd',
          rules: const RulesView(sets: [], mode: null),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        find.descendant(
          of: find.byKey(const Key('mode-row')),
          matching: find.text('Своё'),
        ),
        findsOneWidget,
      );
    });

    testWidgets('сеть неизвестна — так и сказано', (tester) async {
      await pumpApp(tester, rig: rigWith(idleState()));
      expect(find.text('не определена'), findsOneWidget);
    });

    testWidgets('пресет выбирается списком и уходит в ядро как select_mode', (
      tester,
    ) async {
      final rig = await pumpApp(tester, rig: rigWith(idleState()));

      await tester.tap(find.byKey(const Key('mode-row')));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('mode-option-work')), findsOneWidget);
      expect(find.byKey(const Key('mode-option-home')), findsOneWidget);

      await tester.tap(find.byKey(const Key('mode-option-home')));
      await tester.pumpAndSettle();
      expect(rig.bridge.rulesChanges, [const RulesChange.selectMode('home')]);
      expect(
        find.descendant(
          of: find.byKey(const Key('mode-row')),
          matching: find.text('Дом'),
        ),
        findsOneWidget,
      );
    });

    testWidgets(
      'переключатель движка: Авто, AWG, CSQTT; закрепить и вернуть «Авто»',
      (tester) async {
        final rig = await pumpApp(tester, rig: rigWith(idleState()));

        final switcher = find.byKey(const Key('engine-switch'));
        for (final label in ['Авто', 'AWG', 'CSQTT']) {
          expect(
            find.descendant(of: switcher, matching: find.text(label)),
            findsOneWidget,
          );
        }
        SegmentedButton<String> current() => tester.widget(switcher);
        expect(current().selected, {'auto'});

        await tester.tap(
          find.descendant(of: switcher, matching: find.text('CSQTT')),
        );
        await tester.pumpAndSettle();
        expect(rig.bridge.engineModes, [const EngineMode.pinned('csqtt')]);
        expect(current().selected, {'csqtt'});

        await tester.tap(
          find.descendant(of: switcher, matching: find.text('Авто')),
        );
        await tester.pumpAndSettle();
        expect(rig.bridge.engineModes.last, const EngineMode.auto());
        expect(current().selected, {'auto'});
      },
    );

    testWidgets('если в подписке нет движков, переключателя нет', (
      tester,
    ) async {
      await pumpApp(tester, rig: rigWith(idleState(engines: const [])));
      expect(find.byKey(const Key('engine-switch')), findsNothing);
    });
  });

  group('«+ Домен»', () {
    testWidgets('URL из буфера → «Добавить site.com?» → домен в правилах', (
      tester,
    ) async {
      final rig = await pumpApp(
        tester,
        rig: rigWith(
          idleState(),
          clipboard: FakeClipboard('https://www.site.com/page?utm=1'),
        ),
      );

      await tester.tap(find.byKey(const Key('add-domain-button')));
      await tester.pumpAndSettle();
      expect(find.text('Добавить site.com?'), findsOneWidget);

      await tester.tap(find.byKey(const Key('confirm-domain-yes')));
      await tester.pumpAndSettle();
      expect(rig.bridge.rulesChanges, [
        const RulesChange.addUserDomain('site.com'),
      ]);
      expect(find.text('Добавлено: site.com'), findsOneWidget);
    });

    testWidgets('«Отмена» ничего не добавляет', (tester) async {
      final rig = await pumpApp(
        tester,
        rig: rigWith(idleState(), clipboard: FakeClipboard('site.com')),
      );
      await tester.tap(find.byKey(const Key('add-domain-button')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Отмена'));
      await tester.pumpAndSettle();
      expect(rig.bridge.rulesChanges, isEmpty);
    });

    testWidgets('в буфере нет домена — подсказка вместо диалога', (
      tester,
    ) async {
      await pumpApp(
        tester,
        rig: rigWith(idleState(), clipboard: FakeClipboard('привет')),
      );
      await tester.tap(find.byKey(const Key('add-domain-button')));
      await tester.pumpAndSettle();
      expect(find.text('В буфере нет ссылки или домена'), findsOneWidget);
      expect(find.byKey(const Key('confirm-domain')), findsNothing);
    });
  });

  group('баннеры', () {
    testWidgets('обновление: «Скачать» открывает сборку, закрытие помнится', (
      tester,
    ) async {
      final rig = await pumpApp(
        tester,
        rig: rigWith(
          idleState(
            banners: const [
              UpdateAvailableBanner(
                latest: '0.2.0',
                url: 'https://example.org/Braid.zip',
              ),
            ],
          ),
        ),
      );
      expect(find.text('Доступна версия 0.2.0'), findsOneWidget);

      await tester.tap(find.byKey(const Key('banner-update-open')));
      await tester.pumpAndSettle();
      expect(rig.urls.opened, ['https://example.org/Braid.zip']);

      await tester.tap(find.byKey(const Key('banner-update-close')));
      await tester.pumpAndSettle();
      expect(find.text('Доступна версия 0.2.0'), findsNothing);
      expect(rig.prefs.updateDismissed, '0.2.0');
    });

    testWidgets('обновление без ссылки на сборку: кнопки «Скачать» нет', (
      tester,
    ) async {
      await pumpApp(
        tester,
        rig: rigWith(
          idleState(banners: const [UpdateAvailableBanner(latest: '0.2.0')]),
        ),
      );
      expect(find.text('Доступна версия 0.2.0'), findsOneWidget);
      expect(find.byKey(const Key('banner-update-open')), findsNothing);
    });

    testWidgets('ссылка на сборку не открылась — сказано', (tester) async {
      final rig = rigWith(
        idleState(
          banners: const [
            UpdateAvailableBanner(
              latest: '0.2.0',
              url: 'http://insecure.example/x.zip',
            ),
          ],
        ),
      );
      rig.urls.succeeds = false;
      await pumpApp(tester, rig: rig);
      await tester.tap(find.byKey(const Key('banner-update-open')));
      await tester.pumpAndSettle();
      expect(find.text('Не удалось открыть ссылку'), findsOneWidget);
    });

    testWidgets(
      'нужны хеши и вход VK: подсказка и переход в «Движки», если экран есть',
      (tester) async {
        final state = idleState(
          banners: const [NeedsHashesBanner(), NeedsVkLoginBanner()],
        );
        await pumpApp(
          tester,
          rig: rigWith(state),
          extensions: AppExtensions(
            engines: (context) => const Scaffold(body: Text('Экран движков')),
          ),
        );
        expect(find.byKey(const Key('banner-hashes')), findsOneWidget);
        expect(find.byKey(const Key('banner-vk')), findsOneWidget);
        expect(find.textContaining('хеши'), findsWidgets);

        await tester.tap(find.byKey(const Key('banner-hashes-open')));
        await tester.pumpAndSettle();
        expect(find.text('Экран движков'), findsOneWidget);
      },
    );

    testWidgets(
      'без экрана «Движки» баннер хешей остаётся подсказкой без кнопки',
      (tester) async {
        await pumpApp(
          tester,
          rig: rigWith(idleState(banners: const [NeedsHashesBanner()])),
        );
        expect(find.byKey(const Key('banner-hashes')), findsOneWidget);
        expect(find.byKey(const Key('banner-hashes-open')), findsNothing);
      },
    );

    testWidgets('подписка давно не обновлялась: дата и «Обновить»', (
      tester,
    ) async {
      final since = DateTime.utc(2026, 10, 15, 12);
      final rig = await pumpApp(
        tester,
        rig: rigWith(
          idleState(banners: [SubscriptionStaleBanner(since: since)]),
        ),
      );
      expect(find.textContaining('Подписка не обновлялась с'), findsOneWidget);
      expect(find.textContaining('окт'), findsWidgets);

      await tester.tap(find.byKey(const Key('banner-stale-refresh')));
      await tester.pumpAndSettle();
      expect(rig.bridge.calls, contains('refreshSubscription'));
      expect(find.text('Изменений нет — подписка актуальна'), findsOneWidget);
    });

    testWidgets('доступ отозван: сказано прямо', (tester) async {
      await pumpApp(
        tester,
        rig: rigWith(
          idleState(
            banners: const [RevokedBanner()],
            subscription: const SubscriptionView(
              hostId: 'frankfurt',
              hostName: 'Франкфурт',
              version: 7,
              revoked: true,
            ),
          ),
        ),
      );
      expect(find.text('Доступ отозван'), findsOneWidget);
      expect(find.byKey(const Key('banner-revoked-new-link')), findsOneWidget);
    });

    testWidgets('новая сеть: «Сейчас», «Всегда», «Никогда»', (tester) async {
      final state = idleState(
        network: 'wifi:00aa11bb22cc33dd',
        banners: const [AutoconnectAskBanner(network: 'wifi:00aa11bb22cc33dd')],
      );
      final rig = await pumpApp(tester, rig: rigWith(state));
      expect(find.textContaining('Wi‑Fi · 33dd'), findsWidgets);
      expect(find.text('Подключаться к VPN в этой сети?'), findsOneWidget);

      await tester.tap(find.byKey(const Key('banner-autoconnect-always')));
      await tester.pumpAndSettle();
      expect(rig.bridge.settingsWrites.single.autoconnectNetworks, {
        'wifi:00aa11bb22cc33dd': Autoconnect.connect,
      });
      expect(rig.bridge.calls, contains('connect'));
      expect(find.byKey(const Key('banner-autoconnect')), findsNothing);
    });

    testWidgets('новая сеть: «Никогда» запоминает отказ и не подключает', (
      tester,
    ) async {
      final state = idleState(
        network: 'wifi:aa',
        banners: const [AutoconnectAskBanner(network: 'wifi:aa')],
      );
      final rig = await pumpApp(tester, rig: rigWith(state));
      await tester.tap(find.byKey(const Key('banner-autoconnect-never')));
      await tester.pumpAndSettle();
      expect(rig.bridge.settingsWrites.single.autoconnectNetworks, {
        'wifi:aa': Autoconnect.never,
      });
      expect(rig.bridge.calls, isNot(contains('connect')));
    });

    testWidgets('нечитаемый кадр ядра — плашка с причиной, окно живёт', (
      tester,
    ) async {
      final rig = await pumpApp(tester, rig: rigWith(idleState()));
      rig.bridge.emitBroken('поле «kind»: ожидалась строка');
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('banner-protocol')), findsOneWidget);
      expect(find.textContaining('kind'), findsOneWidget);
      expect(find.text('Отключено'), findsOneWidget);
    });
  });

  group('правило sudoers на macOS', () {
    testWidgets(
      'первое подключение предлагает «Не спрашивать пароль», показывает команду',
      (tester) async {
        final bridge = FakeBridge(privilege: rule);
        final rig = await pumpApp(
          tester,
          rig: rigWith(idleState(), bridge: bridge),
        );

        await tester.tap(connectButton);
        await openDialog(tester);
        expect(find.text('Не спрашивать пароль в дальнейшем?'), findsOneWidget);
        expect(find.text(sudoersPlanFixture.command), findsOneWidget);
        expect(find.text(sudoersPlanFixture.rule), findsOneWidget);
        expect(
          rig.bridge.appliedSudoers,
          isEmpty,
          reason: 'без согласия ничего не ставится',
        );
        expect(
          rig.bridge.calls,
          isNot(contains('connect')),
          reason: 'подключение ждёт ответа',
        );

        await tester.tap(find.byKey(const Key('sudoers-install')));
        await tester.pumpAndSettle();
        expect(rig.bridge.appliedSudoers, [sudoersPlanFixture.command]);
        expect(rig.bridge.calls, contains('connect'));
      },
    );

    testWidgets(
      'пока ждём ответа на вопрос о правиле, состояние уже «Подключаюсь…», кнопка отменяет',
      (tester) async {
        final bridge = FakeBridge(privilege: rule);
        final rig = await pumpApp(
          tester,
          rig: rigWith(idleState(), bridge: bridge),
        );

        await tester.tap(connectButton);
        await openDialog(tester);
        expect(find.byKey(const Key('sudoers-offer')), findsOneWidget);
        expect(find.text('Подключаюсь…'), findsOneWidget);
        expect(inConnect('Отменить'), findsOneWidget);
        expect(rig.bridge.calls, isNot(contains('connect')));
      },
    );

    testWidgets('«Не сейчас» — подключение идёт дальше, правило не ставится', (
      tester,
    ) async {
      final bridge = FakeBridge(privilege: rule);
      final rig = await pumpApp(
        tester,
        rig: rigWith(idleState(), bridge: bridge),
      );

      await tester.tap(connectButton);
      await openDialog(tester);
      await tester.tap(find.byKey(const Key('sudoers-later')));
      await tester.pumpAndSettle();

      expect(rig.bridge.appliedSudoers, isEmpty);
      expect(rig.bridge.calls, contains('connect'));
      expect(rig.prefs.sudoersDeclined, isTrue);
    });

    testWidgets('после обновления приложения: баннер и та же процедура', (
      tester,
    ) async {
      final bridge = FakeBridge(
        privilege: const PrivilegeStatus(
          supported: true,
          ruleInstalled: true,
          helperCurrent: false,
        ),
      );
      final rig = await pumpApp(
        tester,
        rig: rigWith(idleState(), bridge: bridge),
      );

      expect(
        find.text('Обновить правило для нового помощника'),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const Key('banner-sudoers-update')));
      await tester.pumpAndSettle();

      expect(
        find.text('Обновить правило для нового помощника?'),
        findsOneWidget,
      );
      expect(find.text(sudoersPlanFixture.command), findsOneWidget);
      await tester.tap(find.byKey(const Key('sudoers-install')));
      await tester.pumpAndSettle();

      expect(rig.bridge.appliedSudoers, [sudoersPlanFixture.command]);
      expect(find.text('Обновить правило для нового помощника'), findsNothing);
    });

    testWidgets('правило актуально — ни баннера, ни вопросов', (tester) async {
      final bridge = FakeBridge(
        privilege: const PrivilegeStatus(
          supported: true,
          ruleInstalled: true,
          helperCurrent: true,
        ),
      );
      final rig = await pumpApp(
        tester,
        rig: rigWith(idleState(), bridge: bridge),
      );
      expect(find.byKey(const Key('banner-sudoers')), findsNothing);
      await tester.tap(connectButton);
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('sudoers-offer')), findsNothing);
      expect(rig.bridge.calls, contains('connect'));
    });
  });
}
