# Local rmux telemetry

The runtime exports aggregate metrics only when `RMUX_METRICS_ADDR` is set.
For a host-only check, launch rmux with `RMUX_METRICS_ADDR=127.0.0.1:9464`
and run `curl --fail http://127.0.0.1:9464/metrics` in a second terminal.

## Docker Compose on Linux

Prometheus reaches the host through `host.docker.internal:9464`. A loopback
listener cannot accept connections through Docker's bridge gateway. Bind rmux
to the host's default Docker bridge address, obtained from Docker itself:

```sh
docker compose -f observability/compose.yaml up -d
rmux_gateway=$(docker network inspect bridge --format '{{(index .IPAM.Config 0).Gateway}}')
RMUX_METRICS_ADDR="${rmux_gateway}:9464" rmux
```

Run this from the repository root with the built `rmux` on `PATH` (or substitute
`./target/debug/rmux`). Keep rmux running while viewing the dashboard. This bind
is reachable by local containers without exposing metrics on a LAN interface.
If the Docker daemon uses a customized host gateway, bind to that address
instead. Rootless container networking and Docker Desktop are not verified.
To monitor an rmux container instead, set its metrics bind to `0.0.0.0:9464`
inside the container and change the scrape target to its Compose service name.
Do not publish that container's metrics port to the LAN.

Open [Grafana](http://127.0.0.1:3000/d/rmux-runtime) for the provisioned,
read-only dashboard and [Prometheus targets](http://127.0.0.1:9090/targets)
to check scrape connectivity. Both UI ports bind to host loopback. Grafana uses
anonymous Viewer access for this local stack. No passwords are stored in the
repository. Prometheus retains seven days of samples in a named volume.

The dashboard shows exporter availability, uptime, pane count, PTY byte rate,
frame rate, and error rate. Rates need at least two successful scrapes. Counter
resets on process restart are handled by `rate()`. Missing samples display as
no data rather than a fabricated zero. Only the five documented rmux metrics
are retained; relabeling drops all exporter labels. Do not add shell, command,
directory, pane-content, or session identifiers as labels.

```sh
docker compose -f observability/compose.yaml config --quiet
docker compose -f observability/compose.yaml logs --tail=100
docker compose -f observability/compose.yaml down
# To explicitly discard stored metrics and Grafana state:
docker compose -f observability/compose.yaml down --volumes
```

Versions are pinned to release tags. Update images through dependency review;
these tags are not immutable content digests. This is a local development
stack, not a production monitoring deployment.
