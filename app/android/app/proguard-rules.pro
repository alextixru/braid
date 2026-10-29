# JNI-контракт с Rust (core/braid-platform/src/android): Rust зовёт методы сервиса по имени
# (protect, requestEstablish, notify, updateNotification, setAutostart), а native-методы
# связываются по именам Java_org_braid_vpn_BraidVpnService_* — ничего не переименовывать
# и не выбрасывать.
-keep class org.braid.vpn.BraidVpnService { *; }
-keepclasseswithmembernames class * {
    native <methods>;
}
