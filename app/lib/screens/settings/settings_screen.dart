import 'package:braid/app.dart' show appName;
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/settings.dart';
import 'package:braid/util/format.dart';
import 'package:braid/widgets/page_body.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

/// «Настройки»: автозапуск, автоподключение по сетям, уведомления о
/// переключении (выкл. по умолчанию), kill switch, режим DNS, правило sudoers
/// на macOS, версия (ист. 55, 58, 59, 33, 77, 7).
class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final settings = controller.settings;
    final theme = Theme.of(context);

    return Scaffold(
      appBar: AppBar(title: const Text('Настройки')),
      body: PageBody(
        children: [
          Card(
            child: Column(
              children: [
                SwitchListTile(
                  key: const Key('setting-autostart'),
                  title: const Text('Автозапуск'),
                  subtitle: Text(
                    controller.platform.isAndroid
                        ? 'Возвращать подключение после перезагрузки телефона'
                        : 'Запускать $appName при входе в систему и возвращать подключение',
                  ),
                  value: settings.autostart,
                  onChanged: (on) => controller.updateSettings(
                    (s) => s.copyWith(autostart: on),
                  ),
                ),
                const Divider(height: 1),
                SwitchListTile(
                  key: const Key('setting-notify'),
                  title: const Text('Уведомлять о переключении движка'),
                  subtitle: const Text(
                    'Например: «AWG недоступен (потери 70 %), перешёл на CSQTT». '
                    'По умолчанию выключено.',
                  ),
                  value: settings.notifySwitches,
                  onChanged: (on) => controller.updateSettings(
                    (s) => s.copyWith(notifySwitches: on),
                  ),
                ),
                const Divider(height: 1),
                SwitchListTile(
                  key: const Key('setting-kill-switch'),
                  title: const Text('Kill switch'),
                  subtitle: const Text(
                    'Пока подключено, но ни один движок не работает, трафик по '
                    'правилам блокируется и не уходит напрямую.',
                  ),
                  value: settings.killSwitch,
                  onChanged: (on) => controller.updateSettings(
                    (s) => s.copyWith(killSwitch: on),
                  ),
                ),
              ],
            ),
          ),
          const SectionHeader('Режим DNS'),
          SizedBox(
            width: double.infinity,
            child: SegmentedButton<DnsMode>(
              key: const Key('setting-dns-mode'),
              showSelectedIcon: false,
              segments: const [
                ButtonSegment(
                  value: DnsMode.rulesOnly,
                  label: Text('Только домены правил'),
                ),
                ButtonSegment(
                  value: DnsMode.allViaTunnel,
                  label: Text('Весь DNS через туннель'),
                ),
              ],
              selected: {settings.dnsMode},
              onSelectionChanged: (selection) => controller.updateSettings(
                (s) => s.copyWith(dnsMode: selection.first),
              ),
            ),
          ),
          const SizedBox(height: 8),
          Text(
            settings.dnsMode == DnsMode.rulesOnly
                ? 'Через туннель идут только запросы доменов из правил; остальное — '
                      'напрямую, как обычно.'
                : 'Провайдер не видит ни одного запроса, но сайты могут отвечать так, '
                      'будто ты в стране хоста (гео-ответы CDN).',
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
          const SectionHeader(
            'Автоподключение по сети',
            subtitle:
                'Что делать, когда ты оказываешься в сети, а VPN выключен.',
          ),
          SizedBox(
            width: double.infinity,
            child: SegmentedButton<Autoconnect>(
              key: const Key('autoconnect-default'),
              showSelectedIcon: false,
              segments: [
                for (final rule in Autoconnect.values)
                  ButtonSegment(value: rule, label: Text(_ruleLabel(rule))),
              ],
              selected: {settings.autoconnectDefault},
              onSelectionChanged: (selection) => controller.updateSettings(
                (s) => s.copyWith(autoconnectDefault: selection.first),
              ),
            ),
          ),
          const SizedBox(height: 4),
          Text(
            'Это правило для новых сетей. Для каждой известной можно выбрать своё.',
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: 12),
          _KnownNetworks(controller: controller),
          if (controller.privilege?.supported ?? false)
            _SudoersTile(controller: controller),
          const SectionHeader('О версии'),
          Card(
            child: ListTile(
              key: const Key('setting-version'),
              leading: const Icon(Icons.info_outline),
              title: Text('$appName ${controller.clientVersion}'),
              subtitle: Text('ядро ${controller.coreVersion}'),
            ),
          ),
        ],
      ),
    );
  }
}

String _ruleLabel(Autoconnect rule) => switch (rule) {
  Autoconnect.connect => 'Подключаться',
  Autoconnect.ask => 'Спрашивать',
  Autoconnect.never => 'Нет',
};

class _KnownNetworks extends StatelessWidget {
  const _KnownNetworks({required this.controller});

  final BraidController controller;

  @override
  Widget build(BuildContext context) {
    final settings = controller.settings;
    final current = controller.state.network;
    final ids = [
      ?current,
      for (final id in settings.autoconnectNetworks.keys)
        if (id != current) id,
    ];
    if (ids.isEmpty) {
      return Text(
        'Сети появятся здесь, когда ты в них побываешь.',
        key: const Key('networks-empty'),
        style: Theme.of(context).textTheme.bodyMedium,
      );
    }
    return Card(
      child: Column(
        children: [
          for (var i = 0; i < ids.length; i++) ...[
            if (i > 0) const Divider(height: 1),
            _NetworkTile(
              id: ids[i],
              isCurrent: ids[i] == current,
              rule:
                  settings.autoconnectNetworks[ids[i]] ??
                  settings.autoconnectDefault,
              controller: controller,
            ),
          ],
        ],
      ),
    );
  }
}

class _NetworkTile extends StatelessWidget {
  const _NetworkTile({
    required this.id,
    required this.isCurrent,
    required this.rule,
    required this.controller,
  });

  final String id;
  final bool isCurrent;
  final Autoconnect rule;
  final BraidController controller;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: const Icon(Icons.wifi),
      title: Text(networkLabel(id)),
      subtitle: isCurrent ? const Text('сейчас') : null,
      trailing: PopupMenuButton<Autoconnect>(
        key: Key('network-rule-$id'),
        tooltip: 'Что делать в этой сети',
        initialValue: rule,
        onSelected: (chosen) => controller.updateSettings(
          (s) => s.copyWith(
            autoconnectNetworks: {...s.autoconnectNetworks, id: chosen},
          ),
        ),
        itemBuilder: (context) => [
          for (final option in Autoconnect.values)
            PopupMenuItem(
              key: Key('network-rule-$id-${option.name}'),
              value: option,
              child: Text(_ruleLabel(option)),
            ),
        ],
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [Text(_ruleLabel(rule)), const Icon(Icons.arrow_drop_down)],
        ),
      ),
    );
  }
}

class _SudoersTile extends StatelessWidget {
  const _SudoersTile({required this.controller});

  final BraidController controller;

  @override
  Widget build(BuildContext context) {
    final status = controller.privilege!;
    final outdated = status.ruleInstalled && !status.helperCurrent;
    final subtitle = !status.ruleInstalled
        ? 'Правило не установлено — пароль администратора спрашивается при каждом подключении.'
        : outdated
        ? 'Правило устарело: приложение обновилось. Обнови его, и пароль снова не понадобится.'
        : 'Правило установлено — подключение без пароля.';
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SectionHeader('Пароль администратора'),
        Card(
          child: ListTile(
            key: const Key('setting-sudoers'),
            leading: const Icon(Icons.admin_panel_settings_outlined),
            title: const Text('Не спрашивать пароль'),
            subtitle: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(subtitle),
                if (!status.ruleInstalled || outdated) ...[
                  const SizedBox(height: 8),
                  FilledButton.tonal(
                    key: const Key('setting-sudoers-install'),
                    onPressed: controller.installSudoersRule,
                    child: Text(outdated ? 'Обновить' : 'Установить'),
                  ),
                ],
              ],
            ),
            trailing: status.ruleInstalled && !outdated
                ? const Icon(Icons.check_circle_outline)
                : null,
          ),
        ),
      ],
    );
  }
}
