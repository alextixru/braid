package org.braid.vpn

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.telephony.TelephonyManager
import java.net.Inet4Address

/**
 * Физическая сеть под VPN: `NetworkCallback` по сетям с интернетом и без VPN (наш
 * собственный интерфейс не считается). Из лучшей сети (Ethernet > Wi-Fi > сотовая,
 * проверенная выше непроверенной) берутся вид, устойчивый идентификатор и DNS из
 * `LinkProperties` — исходные резолверы для запросов не по правилам.
 */
class NetworkWatcher(
    context: Context,
    private val onChanged: (id: String, kind: String) -> Unit,
    private val onLost: () -> Unit,
    private val onDns: (List<String>) -> Unit,
) {
    private class Info(var caps: NetworkCapabilities? = null, var props: LinkProperties? = null)

    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val telephony = context.getSystemService(TelephonyManager::class.java)
    private val networks = LinkedHashMap<Network, Info>()
    private var current: Pair<String, String>? = null
    private var currentDns: List<String>? = null

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = update(network) {}

        override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) =
            update(network) { it.caps = networkCapabilities }

        override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) =
            update(network) { it.props = linkProperties }

        override fun onLost(network: Network) {
            synchronized(this@NetworkWatcher) { networks.remove(network) }
            recompute()
        }
    }

    fun start() {
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        connectivity.registerNetworkCallback(request, callback)
    }

    fun stop() {
        try {
            connectivity.unregisterNetworkCallback(callback)
        } catch (_: IllegalArgumentException) {
            // уже снят
        }
    }

    private fun update(network: Network, change: (Info) -> Unit) {
        synchronized(this) { change(networks.getOrPut(network) { Info() }) }
        recompute()
    }

    private fun recompute() {
        val best = synchronized(this) {
            networks.values.filter { it.caps != null }.maxByOrNull { score(it.caps!!) }
        }
        if (best == null) {
            if (current != null) {
                current = null
                currentDns = null
                onLost()
            }
            return
        }
        val caps = best.caps!!
        val props = best.props
        val kind = kindOf(caps)
        val dns = props?.dnsServers?.mapNotNull { it.hostAddress } ?: emptyList()
        val gateway = props?.routes
            ?.firstOrNull { it.destination.prefixLength == 0 && it.destination.address is Inet4Address }
            ?.gateway?.hostAddress
        val parts = if (kind == "cellular") {
            listOf(telephony?.networkOperator.orEmpty())
        } else {
            listOf(gateway.orEmpty()) + dns
        }
        val next = NetworkIdentity.id(kind, parts) to kind
        if (next != current) {
            current = next
            onChanged(next.first, kind)
        }
        if (dns != currentDns) {
            currentDns = dns
            onDns(dns)
        }
    }

    private fun score(caps: NetworkCapabilities): Int {
        val transport = when {
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> 3
            caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> 2
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> 1
            else -> 0
        }
        val validated = if (caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)) 10 else 0
        return transport + validated
    }

    /** Вид сети для ядра: `wifi | cellular | wired | other` (`NetworkKind`). */
    private fun kindOf(caps: NetworkCapabilities): String = when {
        caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> "wifi"
        caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> "cellular"
        caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> "wired"
        else -> "other"
    }
}
