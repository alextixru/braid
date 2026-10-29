/// Значение с сравнением по содержимому: модели на границе с мостом
/// сравниваются в тестах и при дедупликации событий, а не по ссылке.
mixin Value {
  /// Поля, по которым сравнивается значение.
  List<Object?> get props;

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other.runtimeType == runtimeType &&
          other is Value &&
          deepEquals(props, other.props);

  @override
  int get hashCode => Object.hashAll(props.map(_deepHash));
}

/// Глубокое сравнение списков, словарей и значений.
bool deepEquals(Object? a, Object? b) {
  if (identical(a, b)) return true;
  if (a is List && b is List) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (!deepEquals(a[i], b[i])) return false;
    }
    return true;
  }
  if (a is Map && b is Map) {
    if (a.length != b.length) return false;
    for (final key in a.keys) {
      if (!b.containsKey(key) || !deepEquals(a[key], b[key])) return false;
    }
    return true;
  }
  return a == b;
}

int _deepHash(Object? value) {
  if (value is List) return Object.hashAll(value.map(_deepHash));
  if (value is Map) {
    return Object.hashAllUnordered(
      value.entries.map(
        (e) => Object.hash(_deepHash(e.key), _deepHash(e.value)),
      ),
    );
  }
  return value.hashCode;
}
