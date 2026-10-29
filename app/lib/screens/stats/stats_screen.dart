/// Экран «Статистика» (ист. 63, A03): только админ — переключения за 7 дней,
/// доля времени по движкам, по сетям (где что падало) и экспорт файла.
///
/// Данные — журнал устройства админа, сводка считается здесь по файлу
/// `export_stats`. У остальных пользователей экрана нет в меню, а если он всё
/// же открыт (права сняли), вместо данных — объяснение, и журнал не читается.
library;

import 'dart:async';

import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/stats/stats_report.dart';
import 'package:braid/screens/stats/stats_texts.dart';
import 'package:braid/util/format.dart';
import 'package:braid/widgets/page_body.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

/// Подключается как `AppExtensions.stats`.
Widget buildStatsScreen(BuildContext context) => const StatsScreen();

class StatsScreen extends StatefulWidget {
  const StatsScreen({super.key});

  @override
  State<StatsScreen> createState() => _StatsScreenState();
}

class _StatsScreenState extends State<StatsScreen> {
  EngineServices? _services;
  StatsReport? _report;
  String? _error;
  bool _loading = false;
  bool _requested = false;
  bool _exporting = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final controller = BraidScope.of(context);
    _services ??= EngineServices.of(BraidScope.read(context));
    if (!controller.state.admin) {
      // Права сняли: показанное уходит с экрана и перечитается при возврате.
      _requested = false;
      _report = null;
      _error = null;
    } else if (!_requested) {
      _requested = true;
      unawaited(_load());
    }
  }

  Future<void> _load() async {
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final report = await _services!.stats.load();
      if (mounted) setState(() => _report = report);
    } on Object catch (error) {
      if (mounted) {
        setState(() => _error = BraidController.describeError(error));
      }
    } finally {
      if (mounted) setState(() => _loading = false);
    }
  }

  Future<void> _export(BraidController controller) async {
    setState(() => _exporting = true);
    try {
      final saved = await _services!.stats.export();
      if (saved != null) controller.showNotice('Сохранено: $saved');
    } on Object catch (error) {
      controller.showNotice(BraidController.describeError(error));
    } finally {
      if (mounted) setState(() => _exporting = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final admin = controller.state.admin;
    return Scaffold(
      appBar: AppBar(
        title: const Text('Статистика'),
        actions: [
          if (admin)
            IconButton(
              key: const Key('stats-refresh'),
              tooltip: 'Обновить',
              icon: const Icon(Icons.refresh),
              onPressed: _loading ? null : _load,
            ),
        ],
      ),
      body: PageBody(
        children: admin
            ? _content(context, controller)
            : [
                Text(
                  'Статистика доступна только администратору',
                  style: Theme.of(context).textTheme.bodyLarge,
                ),
              ],
      ),
    );
  }

  List<Widget> _content(BuildContext context, BraidController controller) {
    final theme = Theme.of(context);
    final report = _report;
    if (_error != null) {
      return [
        Text(
          _error!,
          style: theme.textTheme.bodyLarge?.copyWith(
            color: theme.colorScheme.error,
          ),
        ),
        const SizedBox(height: 12),
        Align(
          alignment: Alignment.centerLeft,
          child: FilledButton.tonal(
            key: const Key('stats-retry'),
            onPressed: _loading ? null : _load,
            child: const Text('Повторить'),
          ),
        ),
      ];
    }
    if (report == null) {
      return const [Center(child: CircularProgressIndicator())];
    }
    final engines = controller.state.engines;
    if (report.events == 0 &&
        report.switches.isEmpty &&
        report.uptime.isEmpty) {
      return [
        Text('За 7 дней событий нет', style: theme.textTheme.titleMedium),
        const SizedBox(height: 4),
        Text(
          'Журнал ведётся на этом устройстве, пока ты подключён.',
          style: theme.textTheme.bodySmall,
        ),
        const SizedBox(height: 16),
        _exportButton(controller),
      ];
    }
    return [
      Card(
        key: const Key('stats-summary'),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('За 7 дней', style: theme.textTheme.titleMedium),
              const SizedBox(height: 4),
              Text('Переключений: ${report.switches.length}'),
              Text('Событий в журнале: ${report.events}'),
            ],
          ),
        ),
      ),
      if (report.uptime.isNotEmpty) ...[
        const SectionHeader('Доля времени по движкам'),
        Card(
          child: Column(
            children: [
              for (final entry in report.uptime.entries)
                _EngineShare(
                  key: Key('stats-engine-${entry.key}'),
                  name: journalEngineName(entry.key, engines),
                  share: report.share(entry.key),
                  time: entry.value,
                ),
            ],
          ),
        ),
      ],
      if (report.networks.isNotEmpty) ...[
        const SectionHeader(
          'По сетям',
          subtitle: 'Где и какой движок работал и откуда приходилось уходить.',
        ),
        for (final network in report.networks) ...[
          _NetworkCard(
            key: Key('stats-network-${network.network}'),
            network: network,
            engines: engines,
          ),
          const SizedBox(height: 8),
        ],
      ],
      if (report.switches.isNotEmpty) ...[
        const SectionHeader('Переключения'),
        Card(
          child: Column(
            children: [
              for (var i = 0; i < report.switches.length; i++) ...[
                if (i > 0) const Divider(height: 1),
                _SwitchTile(
                  key: Key('stats-switch-$i'),
                  entry: report.switches[i],
                  engines: engines,
                  now: controller.now(),
                ),
              ],
            ],
          ),
        ),
      ],
      const SizedBox(height: 16),
      _exportButton(controller),
    ];
  }

  Widget _exportButton(BraidController controller) {
    return Align(
      alignment: Alignment.centerLeft,
      child: FilledButton.tonalIcon(
        key: const Key('stats-export'),
        onPressed: _exporting ? null : () => _export(controller),
        icon: const Icon(Icons.file_download_outlined),
        label: const Text('Экспорт'),
      ),
    );
  }
}

class _EngineShare extends StatelessWidget {
  const _EngineShare({
    super.key,
    required this.name,
    required this.share,
    required this.time,
  });

  final String name;
  final double share;
  final Duration time;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Text(name, style: theme.textTheme.titleSmall),
              const Spacer(),
              Text('${(share * 100).round()} %'),
            ],
          ),
          const SizedBox(height: 6),
          LinearProgressIndicator(value: share),
          const SizedBox(height: 4),
          Text(
            formatSpan(time),
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _NetworkCard extends StatelessWidget {
  const _NetworkCard({super.key, required this.network, required this.engines});

  final NetworkStats network;
  final List<EngineView> engines;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final muted = theme.textTheme.bodySmall?.copyWith(
      color: theme.colorScheme.onSurfaceVariant,
    );
    final time = [
      for (final entry in network.uptime.entries)
        '${journalEngineName(entry.key, engines)} ${formatSpan(entry.value)}',
    ].join(' · ');
    final failures = [
      for (final entry in network.failures.entries)
        '${journalEngineName(entry.key, engines)} — ${entry.value} '
            '${ruPlural(entry.value, 'раз', 'раза', 'раз')}',
    ].join(', ');
    final reasons =
        [
          for (final entry in network.reasons.entries)
            (reasonText(entry.key), entry.value),
        ]..sort((a, b) {
          final byCount = b.$2.compareTo(a.$2);
          return byCount != 0 ? byCount : a.$1.compareTo(b.$1);
        });
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              networkLabel(network.network),
              style: theme.textTheme.titleSmall,
            ),
            const SizedBox(height: 4),
            if (time.isNotEmpty) Text('Время: $time'),
            Text('Переключений: ${network.switches}'),
            if (failures.isNotEmpty) Text('Падал: $failures'),
            if (reasons.isNotEmpty)
              Text(
                'Причины: ${reasons.map((r) => '${r.$1} — ${r.$2}').join(', ')}',
                style: muted,
              ),
          ],
        ),
      ),
    );
  }
}

class _SwitchTile extends StatelessWidget {
  const _SwitchTile({
    super.key,
    required this.entry,
    required this.engines,
    required this.now,
  });

  final SwitchEntry entry;
  final List<EngineView> engines;
  final DateTime now;

  @override
  Widget build(BuildContext context) {
    final metrics = switchMetricsText(
      rttMs: entry.rttMs,
      lossPct: entry.lossPct,
    );
    final when = formatWhen(entry.at.toLocal(), now: now);
    return ListTile(
      dense: true,
      title: Text(
        '${journalEngineName(entry.from, engines)} → '
        '${journalEngineName(entry.to, engines)}',
      ),
      subtitle: Text([when, reasonText(entry.reason), ?metrics].join(' · ')),
    );
  }
}

/// «13 ч 30 мин», «6 ч», «40 мин»; меньше минуты — «меньше минуты».
String formatSpan(Duration span) {
  if (span.inMinutes < 1) return 'меньше минуты';
  return formatDuration(span);
}
