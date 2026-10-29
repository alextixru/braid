import 'dart:async';

import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/screens/engines/engine_texts.dart';
import 'package:braid/util/format.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

/// «Проверить связь» (ист. 74): ручной прогон проб всех движков — отклик,
/// потери, возраст хендшейка. Ядро укладывается в несколько секунд; ждать
/// дольше [limit] экран не станет и скажет об этом.
class ProbeSection extends StatefulWidget {
  const ProbeSection({super.key, this.limit = const Duration(seconds: 10)});

  final Duration limit;

  @override
  State<ProbeSection> createState() => _ProbeSectionState();
}

class _ProbeSectionState extends State<ProbeSection> {
  bool _running = false;
  List<EngineProbeReport>? _reports;
  String? _error;

  Future<void> _run(BraidController controller) async {
    setState(() {
      _running = true;
      _error = null;
    });
    try {
      final reports = await controller.probeNow().timeout(widget.limit);
      if (mounted) setState(() => _reports = reports);
    } on TimeoutException {
      if (mounted) {
        setState(
          () => _error =
              'Проверка не уложилась в ${widget.limit.inSeconds} с — связь '
              'с хостом нестабильна',
        );
      }
    } on Object catch (error) {
      if (mounted) {
        setState(() => _error = BraidController.describeError(error));
      }
    } finally {
      if (mounted) setState(() => _running = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final theme = Theme.of(context);
    final session = controller.state.session;
    // Пробы идут через туннель: без сессии проверять нечего.
    final canProbe =
        session is SessionConnected || session is SessionConnecting;
    final reports = _reports;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const SectionHeader(
          'Проверка связи',
          subtitle: 'Прогоняет пробы всех движков и показывает отчёт.',
        ),
        SizedBox(
          width: double.infinity,
          child: OutlinedButton.icon(
            key: const Key('probe-button'),
            onPressed: canProbe && !_running ? () => _run(controller) : null,
            icon: _running
                ? const SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.network_check),
            label: Text(_running ? 'Проверяю…' : 'Проверить связь'),
          ),
        ),
        if (!canProbe) ...[
          const SizedBox(height: 4),
          Text(
            'Подключись, чтобы проверить связь',
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ],
        if (_error != null) ...[
          const SizedBox(height: 8),
          Text(
            _error!,
            key: const Key('probe-error'),
            style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.error,
            ),
          ),
        ],
        if (reports != null) ...[
          const SizedBox(height: 12),
          Card(
            key: const Key('probe-result'),
            child: Column(
              children: [
                for (var i = 0; i < reports.length; i++) ...[
                  if (i > 0) const Divider(height: 1),
                  _ReportRow(report: reports[i], controller: controller),
                ],
              ],
            ),
          ),
        ],
      ],
    );
  }
}

class _ReportRow extends StatelessWidget {
  const _ReportRow({required this.report, required this.controller});

  final EngineProbeReport report;
  final BraidController controller;

  @override
  Widget build(BuildContext context) {
    final engines = controller.state.engines;
    final known = controller.state.engineById(report.engine);
    // Название и закрепление берём у движка из состояния; отчёт их не несёт.
    final view = EngineView(
      id: report.engine,
      kind: known?.kind ?? report.engine,
      state: report.state,
      pinned: known?.pinned ?? false,
      userAction: known?.userAction,
    );
    final health = healthText(
      rttMs: report.rttMs,
      lossPct: report.lossPct,
      handshakeAgeS: report.handshakeAgeS,
    );
    return ListTile(
      key: Key('probe-${report.engine}'),
      title: Text(engineLabel(view, engines)),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(engineStatusText(view, active: false)),
          ?(health == null ? null : Text(health)),
        ],
      ),
    );
  }
}
