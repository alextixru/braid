import 'package:braid/controller/braid_controller.dart';
import 'package:braid/models/commands.dart';
import 'package:flutter/material.dart';

/// Выбор режима-пресета: список режимов подписки и пояснение про «Своё».
Future<void> showModeSheet(BuildContext context, BraidController controller) {
  return showModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    isScrollControlled: true,
    builder: (sheetContext) => ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        final modes = controller.catalog.modes;
        final selected = controller.state.rules.mode;
        return SafeArea(
          child: ListView(
            shrinkWrap: true,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(24, 0, 24, 8),
                child: Text(
                  'Режим',
                  style: Theme.of(context).textTheme.titleMedium,
                ),
              ),
              for (final mode in modes)
                ListTile(
                  key: Key('mode-option-${mode.id}'),
                  leading: Icon(
                    selected == mode.id
                        ? Icons.radio_button_checked
                        : Icons.radio_button_off,
                  ),
                  title: Text(mode.name),
                  subtitle: Text(_setsText(controller, mode.sets)),
                  onTap: () async {
                    Navigator.pop(sheetContext);
                    final error = await controller.applyRules(
                      RulesChange.selectMode(mode.id),
                    );
                    if (error != null) controller.showNotice(error);
                  },
                ),
              ListTile(
                key: const Key('mode-option-custom'),
                enabled: false,
                leading: Icon(
                  selected == null
                      ? Icons.radio_button_checked
                      : Icons.radio_button_off,
                ),
                title: const Text('Своё'),
                subtitle: const Text(
                  'Своя комбинация наборов — собери её переключателями в «Правилах»',
                ),
              ),
            ],
          ),
        );
      },
    ),
  );
}

String _setsText(BraidController controller, List<String> ids) {
  final names = [
    for (final id in ids) controller.catalog.setById(id)?.name ?? id,
  ];
  return names.isEmpty ? 'Без общих наборов' : names.join(', ');
}
