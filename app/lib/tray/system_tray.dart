/// Значок в строке меню: интерфейс и настоящая реализация на `tray_manager`.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:braid/tray/tray_status.dart';
import 'package:tray_manager/tray_manager.dart' as tm;

/// Значок в строке меню. Тесты подменяют его записывающей подделкой: родной
/// слой `tray_manager` без macOS не проверить.
abstract interface class SystemTray {
  /// Создать значок. [onSelected] получает `TrayMenuItem.id` выбранного пункта.
  Future<void> show({required void Function(String id) onSelected});

  Future<void> setIcon(Uint8List png);

  Future<void> setTooltip(String text);

  Future<void> setMenu(List<TrayMenuItem> items);

  Future<void> dispose();
}

/// `tray_manager` 0.8 (родной API поверх nativeapi). На macOS размер значка по
/// умолчанию 18 pt, цветной (не шаблонный) — как раз то, что рисует
/// `CanvasTrayIconRenderer`.
class TrayManagerTray implements SystemTray {
  tm.TrayIcon? _icon;
  tm.Image? _image;
  _MenuBinding? _menu;
  void Function(String id)? _onSelected;

  @override
  Future<void> show({required void Function(String id) onSelected}) async {
    _onSelected = onSelected;
    final icon = tm.TrayIcon.create();
    if (icon == null) {
      throw StateError('Не удалось создать значок в строке меню');
    }
    // Меню открывается щелчком по значку; Ctrl-щелчок (правая кнопка) — тоже.
    icon.setContextMenuTrigger(tm.ContextMenuTrigger.clicked);
    icon.addListener((event) {
      if (event is tm.TrayIconRightClickedEvent) icon.openContextMenu();
    });
    icon.setVisible(true);
    _icon = icon;
  }

  @override
  Future<void> setIcon(Uint8List png) async {
    final image = tm.Image.fromBase64(base64Encode(png));
    if (image == null) throw ArgumentError('Не удалось прочитать значок');
    _icon?.icon = image;
    _image?.dispose();
    _image = image;
  }

  @override
  Future<void> setTooltip(String text) async => _icon?.setTooltip(text);

  @override
  Future<void> setMenu(List<TrayMenuItem> items) async {
    final icon = _icon;
    if (icon == null) return;
    final binding = _MenuBinding.build(items, (id) => _onSelected?.call(id));
    icon.setContextMenu(binding.menu);
    // Старое меню живёт, пока обрабатывается щелчок, из которого нас вызвали.
    final old = _menu;
    _menu = binding;
    if (old != null) Future<void>.delayed(Duration.zero, old.dispose);
  }

  @override
  Future<void> dispose() async {
    _menu?.dispose();
    _menu = null;
    _image?.dispose();
    _image = null;
    _icon?.dispose();
    _icon = null;
  }
}

/// Родное меню и всё, что должно жить вместе с ним: обёртка nativeapi
/// освобождает ресурс при сборке мусора, а слушатель живёт, пока жив пункт.
class _MenuBinding {
  _MenuBinding._(this.menu);

  final tm.Menu menu;
  final _items = <(tm.MenuItem, tm.ListenerId)>[];
  final _menus = <tm.Menu>[];

  factory _MenuBinding.build(
    List<TrayMenuItem> items,
    void Function(String id) onSelected,
  ) {
    final root = tm.Menu.create();
    if (root == null) throw StateError('Не удалось создать меню');
    final binding = _MenuBinding._(root);
    binding._fill(root, items, onSelected);
    return binding;
  }

  void _fill(
    tm.Menu target,
    List<TrayMenuItem> items,
    void Function(String id) onSelected,
  ) {
    _menus.add(target);
    for (final entry in items) {
      if (entry.isSeparator) {
        target.addSeparator();
        continue;
      }
      final hasChildren = entry.children.isNotEmpty;
      final type = hasChildren
          ? tm.MenuItemType.submenu
          : entry.checked
          ? tm.MenuItemType.checkbox
          : tm.MenuItemType.normal;
      final item = tm.MenuItem.createWithLabelAndType(entry.label, type);
      if (item == null) throw StateError('Не удалось создать пункт меню');
      item.isEnabled = entry.enabled;
      if (type == tm.MenuItemType.checkbox) {
        item.state = tm.MenuItemState.checked;
      }
      if (hasChildren) {
        final submenu = tm.Menu.create();
        if (submenu == null) throw StateError('Не удалось создать подменю');
        _fill(submenu, entry.children, onSelected);
        item.submenu = submenu;
      } else {
        final listener = item.addListener((event) {
          if (event is tm.MenuItemClickedEvent) onSelected(entry.id);
        });
        _items.add((item, listener));
      }
      target.addItem(item);
    }
  }

  void dispose() {
    for (final (item, listener) in _items) {
      item.removeListener(listener);
      item.dispose();
    }
    _items.clear();
    for (final menu in _menus) {
      menu.dispose();
    }
    _menus.clear();
  }
}
