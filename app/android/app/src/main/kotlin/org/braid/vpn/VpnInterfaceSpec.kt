package org.braid.vpn

import org.json.JSONArray
import org.json.JSONObject

/**
 * Одна операция над `VpnService.Builder`. Список операций — и есть описание интерфейса:
 * его проверяет unit-тест, а сервис применяет к настоящему `Builder` один в один.
 */
sealed class BuilderOp {
    data class Address(val address: String, val prefix: Int) : BuilderOp()
    data class Mtu(val mtu: Int) : BuilderOp()
    data class Dns(val server: String) : BuilderOp()
    data class Route(val address: String, val prefix: Int) : BuilderOp()
    /** Режим «по приложениям»: только эти пакеты — в туннель. */
    data class AllowApp(val packageName: String) : BuilderOp()
    /** Режим «по спискам»: эти пакеты всегда напрямую (банки). */
    data class DisallowApp(val packageName: String) : BuilderOp()
    /** `allowFamily(AF_INET6)`: прямой IPv6 не блокируется (правило 9). */
    object AllowIpv6 : BuilderOp()

    fun describe(): String = when (this) {
        is Address -> "addAddress $address/$prefix"
        is Mtu -> "setMtu $mtu"
        is Dns -> "addDnsServer $server"
        is Route -> "addRoute $address/$prefix"
        is AllowApp -> "addAllowedApplication $packageName"
        is DisallowApp -> "addDisallowedApplication $packageName"
        AllowIpv6 -> "allowFamily AF_INET6"
    }
}

/**
 * Описание интерфейса из JSON `requestEstablish` (Rust:
 * `braid_platform::android::config::EstablishRequest`): адрес, MTU, DNS, маршруты
 * (fake-диапазон, подсети правил, адрес DNS-прокси — уже без адресов VK/TURN/DNS ядра),
 * `allowed` или `disallowed`.
 */
class VpnInterfaceSpec private constructor(
    val address: String,
    val prefix: Int,
    val mtu: Int,
    val dns: List<String>,
    val routes: List<Pair<String, Int>>,
    val allowed: List<String>,
    val disallowed: List<String>,
) {
    /** Операции в порядке применения к `Builder`. */
    val ops: List<BuilderOp> = buildList {
        add(BuilderOp.Address(address, prefix))
        add(BuilderOp.Mtu(mtu))
        dns.forEach { add(BuilderOp.Dns(it)) }
        routes.forEach { (net, len) -> add(BuilderOp.Route(net, len)) }
        add(BuilderOp.AllowIpv6)
        allowed.forEach { add(BuilderOp.AllowApp(it)) }
        disallowed.forEach { add(BuilderOp.DisallowApp(it)) }
    }

    /** Читаемое описание — для тестов и журнала. */
    fun describe(): List<String> = ops.map { it.describe() }

    companion object {
        /**
         * @param selfPackage пакет Braid: в режиме «по приложениям» он никогда не попадает в
         * `allowed` — иначе сокеты CSQTT (они не проходят `protect`) ушли бы в собственный туннель.
         */
        fun fromJson(json: String, selfPackage: String): VpnInterfaceSpec {
            val o = JSONObject(json)
            val allowed = strings(o.optJSONArray("allowed")).filter { it != selfPackage }
            val disallowed = strings(o.optJSONArray("disallowed"))
            require(allowed.isEmpty() || disallowed.isEmpty()) {
                "allowed и disallowed одновременно: Builder так не умеет"
            }
            val mtu = o.getInt("mtu")
            require(mtu in 576..65535) { "MTU вне диапазона: $mtu" }
            return VpnInterfaceSpec(
                address = o.getString("address"),
                prefix = o.optInt("prefix", 32),
                mtu = mtu,
                dns = strings(o.optJSONArray("dns")),
                routes = strings(o.optJSONArray("routes")).map(::cidr),
                allowed = allowed,
                disallowed = disallowed,
            )
        }

        private fun strings(array: JSONArray?): List<String> =
            if (array == null) emptyList() else List(array.length()) { array.getString(it) }

        /** `198.18.0.0/15` → (`198.18.0.0`, 15); адрес без префикса — хост `/32`. */
        internal fun cidr(s: String): Pair<String, Int> {
            val slash = s.indexOf('/')
            if (slash < 0) return s to 32
            val prefix = s.substring(slash + 1).toIntOrNull()
            require(prefix != null && prefix in 0..32) { "неверный префикс: $s" }
            return s.substring(0, slash) to prefix
        }
    }
}
