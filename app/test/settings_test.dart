// Экран «Настройки» (ист. 55, 58, 59, 33, 77, 7): автозапуск, автоподключение
// по сетям, уведомления о переключении (по умолчанию выключены), kill switch,
// режим DNS, правило sudoers на macOS, версия.

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/settings.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

Future<Rig> openSettings(
  WidgetTester tester, {
  Settings settings = const Settings(),
  CoreState? state,
  PrivilegeStatus privilege = noPrivilege,
  bool onAndroid = false,
}) async {
  final bridge = FakeBridge(
    state: state ?? idleState(),
    storedSettings: settings,
    privilege: privilege,
    caps: onAndroid ? androidCaps : desktopCaps,
  );
  final rig = Rig(
    bridge: bridge,
    platform: onAndroid ? android : macOS,
    vpn: onAndroid ? FakeVpn() : null,
    autoAnswer: false,
  );
  await pumpApp(tester, rig: rig);
  await openTile(tester, 'tile-settings');
  return rig;
}

SwitchListTile switchTile(WidgetTester tester, String key) =>
    tester.widget<SwitchListTile>(find.byKey(Key(key)));

Settings lastWrite(Rig rig) => rig.bridge.settingsWrites.last;

void main() {
  testWidgets(
    'умолчания: уведомления выключены, kill switch включён, DNS — только домены правил',
    (tester) async {
      await openSettings(tester);

      expect(switchTile(tester, 'setting-autostart').value, isFalse);
      expect(switchTile(tester, 'setting-notify').value, isFalse);
      expect(
        switchTile(tester, 'setting-kill-switch').value,
        isTrue,
        reason: 'G03: защищённое напрямую не уходит, пока пользователь сам не выключит',
      );
      final dns = tester.widget<SegmentedButton<DnsMode>>(
        find.byKey(const Key('setting-dns-mode')),
      );
      expect(dns.selected, {DnsMode.rulesOnly});
      final defaults = tester.widget<SegmentedButton<Autoconnect>>(
        find.byKey(const Key('autoconnect-default')),
      );
      expect(defaults.selected, {Autoconnect.ask});
    },
  );

  testWidgets('настройки, которые уже были, показываются как есть', (
    tester,
  ) async {
    await openSettings(
      tester,
      settings: const Settings(
        autostart: true,
        notifySwitches: true,
        killSwitch: false,
        dnsMode: DnsMode.allViaTunnel,
        autoconnectDefault: Autoconnect.never,
      ),
    );
    expect(switchTile(tester, 'setting-autostart').value, isTrue);
    expect(switchTile(tester, 'setting-notify').value, isTrue);
    expect(switchTile(tester, 'setting-kill-switch').value, isFalse);
    expect(
      tester
          .widget<SegmentedButton<DnsMode>>(
            find.byKey(const Key('setting-dns-mode')),
          )
          .selected,
      {DnsMode.allViaTunnel},
    );
    expect(
      tester
          .widget<SegmentedButton<Autoconnect>>(
            find.byKey(const Key('autoconnect-default')),
          )
          .selected,
      {Autoconnect.never},
    );
  });

  testWidgets('каждый переключатель пишет свою настройку в ядро', (
    tester,
  ) async {
    final rig = await openSettings(tester);

    await tester.tap(find.byKey(const Key('setting-autostart')));
    await tester.pumpAndSettle();
    expect(lastWrite(rig).autostart, isTrue);

    await tester.tap(find.byKey(const Key('setting-notify')));
    await tester.pumpAndSettle();
    expect(lastWrite(rig).notifySwitches, isTrue);
    expect(
      lastWrite(rig).autostart,
      isTrue,
      reason: 'прежние настройки не потерялись',
    );

    // Kill switch включён по умолчанию — явное выключение пишется в ядро.
    await tester.tap(find.byKey(const Key('setting-kill-switch')));
    await tester.pumpAndSettle();
    expect(lastWrite(rig).killSwitch, isFalse);
    expect(rig.bridge.settingsWrites, hasLength(3));
  });

  testWidgets('режим DNS: «весь DNS через туннель» объясняет цену', (
    tester,
  ) async {
    final rig = await openSettings(tester);
    expect(
      find.textContaining('только запросы доменов из правил'),
      findsOneWidget,
    );

    await reveal(tester, find.byKey(const Key('setting-dns-mode')));
    await tester.tap(
      find.descendant(
        of: find.byKey(const Key('setting-dns-mode')),
        matching: find.text('Весь DNS через туннель'),
      ),
    );
    await tester.pumpAndSettle();

    expect(lastWrite(rig).dnsMode, DnsMode.allViaTunnel);
    expect(find.textContaining('гео-ответы'), findsOneWidget);
  });

  testWidgets('умолчание для новых сетей: подключаться, спрашивать, нет', (
    tester,
  ) async {
    final rig = await openSettings(tester);
    final segments = find.byKey(const Key('autoconnect-default'));
    await reveal(tester, segments);

    await tester.tap(
      find.descendant(of: segments, matching: find.text('Подключаться')),
    );
    await tester.pumpAndSettle();
    expect(lastWrite(rig).autoconnectDefault, Autoconnect.connect);

    await tester.tap(find.descendant(of: segments, matching: find.text('Нет')));
    await tester.pumpAndSettle();
    expect(lastWrite(rig).autoconnectDefault, Autoconnect.never);
  });

  testWidgets('известные сети: правило каждой меняется отдельно', (
    tester,
  ) async {
    final rig = await openSettings(
      tester,
      settings: const Settings(
        autoconnectNetworks: {
          'wifi:00aa11bb22cc33dd': Autoconnect.connect,
          'gw:99887766aabbccdd': Autoconnect.never,
        },
      ),
    );
    await reveal(
      tester,
      find.byKey(const Key('network-rule-wifi:00aa11bb22cc33dd')),
    );
    expect(find.text('Wi‑Fi · 33dd'), findsOneWidget);
    expect(find.text('Сеть · ccdd'), findsOneWidget);

    await tester.tap(
      find.byKey(const Key('network-rule-wifi:00aa11bb22cc33dd')),
    );
    await tester.pumpAndSettle();
    await tester.tap(
      find.byKey(const Key('network-rule-wifi:00aa11bb22cc33dd-ask')),
    );
    await tester.pumpAndSettle();

    expect(lastWrite(rig).autoconnectNetworks, {
      'wifi:00aa11bb22cc33dd': Autoconnect.ask,
      'gw:99887766aabbccdd': Autoconnect.never,
    });
  });

  testWidgets('текущая сеть в списке, даже если для неё правила ещё нет', (
    tester,
  ) async {
    final rig = await openSettings(
      tester,
      state: idleState(network: 'wifi:00aa11bb22cc33dd'),
    );
    await reveal(
      tester,
      find.byKey(const Key('network-rule-wifi:00aa11bb22cc33dd')),
    );
    expect(find.textContaining('сейчас'), findsOneWidget);

    await tester.tap(
      find.byKey(const Key('network-rule-wifi:00aa11bb22cc33dd')),
    );
    await tester.pumpAndSettle();
    await tester.tap(
      find.byKey(const Key('network-rule-wifi:00aa11bb22cc33dd-connect')),
    );
    await tester.pumpAndSettle();
    expect(lastWrite(rig).autoconnectNetworks, {
      'wifi:00aa11bb22cc33dd': Autoconnect.connect,
    });
  });

  testWidgets('сетей нет — подсказка вместо пустого списка', (tester) async {
    await openSettings(tester);
    await reveal(tester, find.byKey(const Key('networks-empty')));
    expect(find.byKey(const Key('networks-empty')), findsOneWidget);
  });

  testWidgets('отказ ядра: переключатель возвращается, причина показана', (
    tester,
  ) async {
    final rig = await openSettings(tester);
    rig.bridge.settingsError = const BridgeError(
      kind: BridgeErrorKind.denied,
      message: 'Автозапуск не разрешён системой',
      details: [],
    );
    await tester.tap(find.byKey(const Key('setting-autostart')));
    await tester.pumpAndSettle();
    expect(switchTile(tester, 'setting-autostart').value, isFalse);
    expect(find.text('Автозапуск не разрешён системой'), findsOneWidget);
  });

  testWidgets('версия приложения и ядра', (tester) async {
    await openSettings(tester);
    await reveal(tester, find.byKey(const Key('setting-version')));
    expect(find.textContaining('0.1.0'), findsWidgets);
    expect(
      find.descendant(
        of: find.byKey(const Key('setting-version')),
        matching: find.textContaining('ядро'),
      ),
      findsOneWidget,
    );
  });

  group('пароль администратора (macOS)', () {
    testWidgets('правила нет: статус и «Установить» с показом команды', (
      tester,
    ) async {
      final rig = await openSettings(
        tester,
        privilege: const PrivilegeStatus(
          supported: true,
          ruleInstalled: false,
          helperCurrent: false,
        ),
      );
      await reveal(tester, find.byKey(const Key('setting-sudoers')));
      expect(find.textContaining('не установлено'), findsOneWidget);

      await tester.tap(find.byKey(const Key('setting-sudoers-install')));
      await tester.pumpAndSettle();
      expect(find.text(sudoersPlanFixture.command), findsOneWidget);
      await tester.tap(find.byKey(const Key('sudoers-install')));
      await tester.pumpAndSettle();

      expect(rig.bridge.appliedSudoers, [sudoersPlanFixture.command]);
      await reveal(tester, find.byKey(const Key('setting-sudoers')));
      expect(find.textContaining('установлено'), findsOneWidget);
    });

    testWidgets('помощник обновился: правило устарело, кнопка «Обновить»', (
      tester,
    ) async {
      await openSettings(
        tester,
        privilege: const PrivilegeStatus(
          supported: true,
          ruleInstalled: true,
          helperCurrent: false,
        ),
      );
      await reveal(tester, find.byKey(const Key('setting-sudoers')));
      expect(find.textContaining('устарело'), findsOneWidget);
      expect(
        find.descendant(
          of: find.byKey(const Key('setting-sudoers-install')),
          matching: find.text('Обновить'),
        ),
        findsOneWidget,
      );
    });

    testWidgets('не macOS — блока нет', (tester) async {
      await openSettings(tester, onAndroid: true);
      expect(find.byKey(const Key('setting-sudoers')), findsNothing);
    });

    testWidgets('платформа без sudoers — блока нет', (tester) async {
      await openSettings(tester);
      expect(find.byKey(const Key('setting-sudoers')), findsNothing);
    });
  });

  testWidgets('Android: подпись автозапуска говорит про перезагрузку', (
    tester,
  ) async {
    await openSettings(tester, onAndroid: true);
    expect(find.textContaining('перезагрузки'), findsOneWidget);
  });
}
