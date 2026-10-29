import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/screens/rules/apps_section.dart';
import 'package:braid/screens/rules/user_rules_section.dart';
import 'package:braid/util/format.dart';
import 'package:braid/widgets/page_body.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

/// «Правила»: режим-пресет, общие наборы с переключателями и исключениями,
/// свои домены и подсети, приложения (Android), перенос файлом.
class RulesScreen extends StatelessWidget {
  const RulesScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final rules = controller.state.rules;
    return Scaffold(
      appBar: AppBar(title: const Text('Правила')),
      body: PageBody(
        children: [
          const SectionHeader(
            'Режим',
            subtitle:
                'Пресет — положение переключателей ниже. Изменишь один '
                'переключатель, и режим станет «Своё».',
          ),
          _ModeChips(controller: controller),
          const SectionHeader(
            'Общие наборы',
            subtitle: 'Домены и подсети, которые идут через VPN. Каждый набор — отдельно.',
          ),
          Card(
            child: Column(
              children: [
                for (var i = 0; i < rules.sets.length; i++) ...[
                  if (i > 0) const Divider(height: 1),
                  _SetTile(
                    set: rules.sets[i],
                    entry: controller.catalog.setById(rules.sets[i].id),
                    controller: controller,
                  ),
                ],
                if (rules.sets.isEmpty)
                  const ListTile(title: Text('В подписке нет наборов')),
              ],
            ),
          ),
          const UserRulesSection(),
          if (controller.caps.perApp) const AppsSection(),
          if (controller.files.available) const _TransferSection(),
        ],
      ),
    );
  }
}

class _ModeChips extends StatelessWidget {
  const _ModeChips({required this.controller});

  final BraidController controller;

  @override
  Widget build(BuildContext context) {
    final selected = controller.state.rules.mode;
    final modes = controller.catalog.modes;
    final current = controller.catalog.modeById(selected);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            for (final mode in modes)
              ChoiceChip(
                key: Key('mode-chip-${mode.id}'),
                label: Text(mode.name),
                selected: selected == mode.id,
                onSelected: (_) async {
                  final error = await controller.applyRules(
                    RulesChange.selectMode(mode.id),
                  );
                  if (error != null) controller.showNotice(error);
                },
              ),
            // «Своё» получается переключателями, выбрать его самому нельзя.
            ChoiceChip(
              key: const Key('mode-chip-custom'),
              label: const Text('Своё'),
              selected: selected == null,
              onSelected: null,
            ),
          ],
        ),
        if (current != null && _hasExclusions(current))
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(
              'В режиме «${current.name}» всегда напрямую: '
              '${[...current.excludeDomains, ...current.excludeIps].join(', ')}',
              key: const Key('mode-exclusions'),
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
      ],
    );
  }

  bool _hasExclusions(ModeEntry mode) =>
      mode.excludeDomains.isNotEmpty || mode.excludeIps.isNotEmpty;
}

class _SetTile extends StatelessWidget {
  const _SetTile({
    required this.set,
    required this.entry,
    required this.controller,
  });

  final RuleSetView set;
  final SetEntry? entry;
  final BraidController controller;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final excluded = [...?entry?.excludeDomains, ...?entry?.excludeIps];
    return SwitchListTile(
      key: Key('set-switch-${set.id}'),
      title: Text(set.name),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(_counts(set)),
          if (excluded.isNotEmpty)
            Text(
              'Всегда напрямую: ${excluded.join(', ')}',
              key: Key('set-exclusions-${set.id}'),
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.tertiary,
              ),
            ),
        ],
      ),
      value: set.enabled,
      onChanged: (on) async {
        final error = await controller.applyRules(
          RulesChange.toggleSet(set.id, on: on),
        );
        if (error != null) controller.showNotice(error);
      },
    );
  }

  String _counts(RuleSetView set) {
    final parts = [
      if (set.domains > 0)
        '${set.domains} ${ruPlural(set.domains, 'домен', 'домена', 'доменов')}',
      if (set.ips > 0)
        '${set.ips} ${ruPlural(set.ips, 'подсеть', 'подсети', 'подсетей')}',
    ];
    return parts.isEmpty ? 'пусто' : parts.join(' · ');
  }
}

class _TransferSection extends StatelessWidget {
  const _TransferSection();

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.read(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SectionHeader(
          'Перенос правил',
          subtitle: 'Свои домены и подсети можно сохранить в файл и вернуть после переустановки.',
        ),
        Wrap(
          spacing: 12,
          runSpacing: 8,
          children: [
            OutlinedButton.icon(
              key: const Key('rules-export'),
              onPressed: controller.exportUserRules,
              icon: const Icon(Icons.file_upload_outlined),
              label: const Text('Экспорт в файл'),
            ),
            OutlinedButton.icon(
              key: const Key('rules-import'),
              onPressed: controller.importUserRules,
              icon: const Icon(Icons.file_download_outlined),
              label: const Text('Импорт из файла'),
            ),
          ],
        ),
      ],
    );
  }
}
