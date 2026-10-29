import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/util/format.dart';
import 'package:flutter/material.dart';

/// «Авто / AWG / CSQTT»: закрепить движок вручную или вернуть автовыбор
/// (ист. 30, 31). Работает и без подключения — выбор запоминает ядро.
class EngineSwitch extends StatelessWidget {
  const EngineSwitch({super.key});

  static const _auto = 'auto';

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final engines = controller.state.engines;
    if (engines.isEmpty) return const SizedBox.shrink();
    final pinned = engines.where((e) => e.pinned);
    return SizedBox(
      width: double.infinity,
      child: SegmentedButton<String>(
        key: const Key('engine-switch'),
        showSelectedIcon: false,
        segments: [
          const ButtonSegment(value: _auto, label: Text('Авто')),
          for (final engine in engines)
            ButtonSegment(
              value: engine.id,
              label: Text(engineLabel(engine, engines)),
            ),
        ],
        selected: {pinned.isEmpty ? _auto : pinned.first.id},
        onSelectionChanged: (selection) {
          final id = selection.first;
          controller.setEngineMode(
            id == _auto ? const EngineMode.auto() : EngineMode.pinned(id),
          );
        },
      ),
    );
  }
}
