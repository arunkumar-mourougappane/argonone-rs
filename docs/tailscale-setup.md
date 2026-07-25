# Tailscale HTTPS setup

`argonone-rs` can serve the web UI over HTTPS using a certificate issued by
Tailscale, scoped to your private tailnet — no port forwarding, no public
domain, no Let's Encrypt account of your own. This is the recommended HTTPS
mode for this audience (homelab/NAS self-hosters); see
[src/https.rs](../src/https.rs) for the alternative `acme` (Let's Encrypt,
public domain) mode.

## What Tailscale actually does

Tailscale is a mesh VPN built on WireGuard. Installing it on a device joins
it to your private "tailnet" — every device gets a stable `100.x.x.x`
address and a MagicDNS hostname (`<device>.<tailnet>.ts.net`), reachable
from every other device in the tailnet regardless of what physical network
each one is actually on. Connections are peer-to-peer and encrypted
end-to-end, punching through NAT/firewalls automatically (falling back to a
relay only when a direct path isn't possible) — **no router configuration
or port forwarding is needed** for devices to reach each other.

Because Tailscale already knows every device's identity, it can also act as
a private CA: `tailscale cert <domain>` gets you a real cert (chained to
Let's Encrypt) scoped to that device's MagicDNS name. That's the mechanism
`argonone-rs`'s `tailscale` HTTPS mode drives.

## Prerequisites

1. **The Pi is joined to your tailnet.**

   ```sh
   sudo tailscale up
   tailscale status
   ```

   `tailscale status` should list the device with a `100.x.x.x` address.

2. **HTTPS Certificates is enabled for your tailnet.** This is off by
   default and is an account-level setting, not something `argonone-rs` or
   the `tailscale` CLI can turn on for you. In the
   [admin console → DNS page](https://login.tailscale.com/admin/dns):
   - Confirm **MagicDNS** is enabled (prerequisite for HTTPS certs).
   - Under **HTTPS Certificates**, click **Enable HTTPS**, then reload the
     page to confirm it actually saved.

   Without this, `tailscale cert` fails with either
   `HTTPS cert support is not enabled/configured for your tailnet` or
   `500 Internal Server Error: your Tailscale account does not support
   getting TLS certs` — both mean the same thing: go back and check this
   step.

3. **`tailscale` CLI is on `argonone-rs`'s `PATH`.** The daemon shells out
   to it directly ([src/https.rs](../src/https.rs)) — if it's installed
   somewhere not on the service's `PATH` (uncommon, but possible under a
   restrictive systemd unit), cert issuance will fail at startup.

## Find your device's domain

```sh
tailscale status --json | grep -i dnsname
```

This prints something like `"DNSName": "euclides.tail75dbb1.ts.net."` —
use the value **without** the trailing dot (that's DNS zone-root notation,
not part of the usable hostname): `euclides.tail75dbb1.ts.net`.

## Validate cert issuance manually, before touching the app

Once HTTPS Certificates is enabled, confirm issuance works standalone —
this isolates any failure (account config, DNS propagation) from an
app-level bug:

```sh
sudo tailscale cert euclides.tail75dbb1.ts.net
```

Success looks like:

```
Wrote public cert to euclides.tail75dbb1.ts.net.crt
Wrote private key to euclides.tail75dbb1.ts.net.key
```

Delete those two files afterward — they were only a sanity check. The app
issues (and manages) its own copy separately.

## Configure `argonone-rs`

In the web UI's **System** page, HTTPS card:

- **Mode**: `Tailscale`
- **Domain**: your MagicDNS name from above (no `https://` prefix, no
  trailing dot) — e.g. `euclides.tail75dbb1.ts.net`
- **Email**: leave blank — only used by `acme` mode.

Save, then **restart the daemon**. Mode switches rebind the listener and
aren't hot-reloaded (unlike cert renewal, which is).

```sh
sudo systemctl restart argonone-rs
```

On success, `journalctl -u argonone-rs` shows:

```
INFO argonone_rs::service: web server listening addr=0.0.0.0:443 mode="tailscale"
Wrote public cert to /var/lib/argonone-rs/tls/fullchain.pem
Wrote private key to /var/lib/argonone-rs/tls/privkey.pem
INFO argonone_rs::https: serving HTTPS via Tailscale-issued certificate domain=...
```

The cert is renewed automatically (checked daily, `tailscale cert` only
actually reissues near expiry) and hot-swapped without a restart. The
System page's HTTPS card also shows the live issuer/expiry and a manual
"Re-issue now" action if you need to force it.

## Validate network reachability

Test from the Pi itself first, bypassing any client-side Tailscale issues:

```sh
curl -vk https://127.0.0.1:443
```

A `303` redirect to `/login` (or your logged-in page) means the server is
healthy. If this **hangs** with no response after the TLS `Client Hello`,
it's not a network problem — see Troubleshooting below.

Then test from the client machine you'll actually browse from (laptop,
phone, etc.) — it must also be joined to the same tailnet:

```sh
tailscale status      # confirms this device is actually connected
ping 100.x.x.x        # the Pi's tailnet IP from `tailscale status` on the Pi
```

100% packet loss here means the *client*, not the Pi, isn't really
connected to the tailnet — re-run `sudo tailscale up` (or the desktop
app's equivalent) on that device and log in again. Once `ping` succeeds,
`https://<device>.<tailnet>.ts.net` should load in the browser.

## Troubleshooting

- **`tailscale cert` fails with "not enabled/configured for your tailnet"
  or the 500 error above** — HTTPS Certificates isn't actually enabled;
  see Prerequisites step 2.
- **Curling `https://127.0.0.1` (or the tailnet domain) on the Pi itself
  hangs indefinitely after the TLS `Client Hello`, with no response at
  all** — this was a real bug (fixed as of the next release after
  v0.6.1): `spawn_server` only set the listener to non-blocking mode in
  plain-HTTP mode, not `tailscale`/`acme` mode, so the async TLS accept
  loop blocked its worker thread forever instead of completing the
  handshake. If you're on an affected version, update — this isn't a
  network or Tailscale-account issue.
- **Everything above passes but the browser still can't reach it** — the
  client device itself isn't joined to the tailnet; see "Validate network
  reachability" above. This is by far the most common cause and is not a
  router or firewall issue — no port forwarding is required for
  Tailscale-to-Tailscale traffic.
