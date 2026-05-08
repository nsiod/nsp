#!/usr/bin/env bash
# End-to-end test for the nsp HTTP API + data plane.
#
# Runs *inside* the tester container on the private docker network
# `nsp-e2e-net`. Exercises every authenticated route, asserts the
# kernel WireGuard backend is live, drives the full peer/user/iptables
# lifecycle, validates input handling and auth invalidation, and
# finally proves the data plane works by bringing up a real
# in-kernel WireGuard interface inside the tester and pinging the
# server through it.
#
# Inputs are environment variables set by docker-compose.yml:
#   NSP_BASE             — http://nsp:8443
#   NSP_ADMIN_PASSWORD   — bootstrap admin password
#   NSP_METRICS_TOKEN    — bearer token for /metrics
#   NSP_SERVER_HOST      — DNS name of the nsp container (for WG endpoint)

set -euo pipefail

NSP_BASE="${NSP_BASE:?NSP_BASE not set}"
NSP_ADMIN_PASSWORD="${NSP_ADMIN_PASSWORD:?NSP_ADMIN_PASSWORD not set}"
NSP_METRICS_TOKEN="${NSP_METRICS_TOKEN:-}"
NSP_SERVER_HOST="${NSP_SERVER_HOST:-nsp}"

# ------------------------------------------------------------------
# Pretty test output. `pass`/`fail`/`assert*` write to stderr so the
# command output stays clean for capture.
# ------------------------------------------------------------------
RED='\033[0;31m'; GREEN='\033[0;32m'; YEL='\033[0;33m'; BLUE='\033[0;34m'; NC='\033[0m'
TESTS_RUN=0
TESTS_FAILED=0

section() { printf "\n${BLUE}### %s ###${NC}\n" "$*" >&2; }
step()    { printf "${YEL}==>${NC} %s\n" "$*" >&2; }
pass()    { printf "${GREEN}OK${NC}    %s\n" "$*" >&2; TESTS_RUN=$((TESTS_RUN+1)); }
fail()    { printf "${RED}FAIL${NC}  %s\n" "$*" >&2; TESTS_RUN=$((TESTS_RUN+1)); TESTS_FAILED=$((TESTS_FAILED+1)); }
die()     { printf "${RED}FATAL${NC} %s\n" "$*" >&2; exit 2; }

assert_eq() {
    if [[ "$1" == "$2" ]]; then
        pass "$3 (=$2)"
    else
        fail "$3: expected=$1 actual=$2"
    fi
}

assert_ne() {
    if [[ "$1" != "$2" ]]; then
        pass "$3"
    else
        fail "$3: expected != $1"
    fi
}

assert_status() {
    if [[ "$1" == "$2" ]]; then
        pass "$3 (HTTP $2)"
    else
        fail "$3: expected HTTP $1, got HTTP $2"
    fi
}

# Poll until predicate returns 0 or timeout (seconds) elapses. Used
# wherever the design says "eventually" — driver lifecycle transitions
# and reconciler convergence specifically.
wait_until() {
    local timeout="$1"; shift
    local label="$1"; shift
    local i
    for ((i = 0; i < timeout * 5; i++)); do
        if "$@"; then
            pass "$label (converged)"
            return 0
        fi
        sleep 0.2
    done
    fail "$label: timed out after ${timeout}s"
    return 1
}

# ------------------------------------------------------------------
# HTTP helpers. `req` returns the body and `req_status` returns the
# HTTP status code. `/tmp/last_body` carries the response body for
# inspection on failure.
# ------------------------------------------------------------------
TOKEN=""

req_status() {
    # $1 method, $2 path, $3? body
    local method="$1" path="$2" body="${3-}"
    local args=(-sS -o /tmp/last_body -w '%{http_code}' -X "$method" "${NSP_BASE}${path}")
    if [[ -n "$TOKEN" ]]; then
        args+=(-H "Authorization: Bearer $TOKEN")
    fi
    if [[ -n "$body" ]]; then
        args+=(-H 'content-type: application/json' --data "$body")
    fi
    curl "${args[@]}" --max-time 10
}

req() {
    local code
    code=$(req_status "$@")
    if [[ "$code" -lt 200 || "$code" -ge 300 ]]; then
        die "request $1 $2 returned HTTP $code: $(cat /tmp/last_body)"
    fi
    cat /tmp/last_body
}

# Refresh JWT — used after password change to obtain a new token.
relogin() {
    local pw="$1"
    local body
    body=$(req POST /api/auth/login "{\"password\":\"$pw\"}")
    TOKEN=$(echo "$body" | jq -r '.token')
    [[ -n "$TOKEN" && "$TOKEN" != "null" ]] || die "relogin failed"
}

# Wait for the API to start. Compose's healthcheck already gates this
# but we keep an extra retry loop for safety in case the tester is
# launched outside compose.
wait_for_api() {
    local i
    for i in {1..30}; do
        if curl --silent -o /dev/null --max-time 2 "${NSP_BASE}/api/healthz" 2>/dev/null; then
            return 0
        fi
        sleep 1
    done
    die "nsp API never became reachable at $NSP_BASE"
}

# Predicate helpers used with wait_until.
wg_running_is() {
    local want="$1"
    [[ "$(req GET /api/protocol/wg/status | jq -r .running)" == "$want" ]]
}
wg_total_peers_is() {
    local want="$1"
    [[ "$(req GET /api/protocol/wg/status | jq -r .total_peers)" == "$want" ]]
}
ss_running_is() {
    local want="$1"
    [[ "$(req GET /api/protocol/ss/status | jq -r .running)" == "$want" ]]
}

# ==================================================================
# Phase 0 — bootstrap
# ==================================================================
section "phase 0 — bootstrap"

step "wait for /api/healthz"
wait_for_api
pass "/api/healthz reachable"

step "GET /api/healthz body shape"
body=$(req GET /api/healthz)
assert_eq "true" "$(echo "$body" | jq -r .ok)" "/api/healthz body.ok"

step "POST /api/auth/login with the bootstrap password"
relogin "$NSP_ADMIN_PASSWORD"
pass "login JWT issued"

step "GET /api/me echoes the admin sub"
me=$(req GET /api/me)
assert_eq "admin" "$(echo "$me" | jq -r .sub)" "/api/me sub"

step "GET /api/status reports versions + driver flags"
status=$(req GET /api/status)
ver=$(echo "$status" | jq -r .version)
if [[ -n "$ver" && "$ver" != "null" ]]; then
    pass "status.version=$ver"
else
    fail "status.version missing"
fi
assert_eq "true" "$(echo "$status" | jq -r .wg_enabled)" "status.wg_enabled"
assert_eq "true" "$(echo "$status" | jq -r .ss_enabled)" "status.ss_enabled"

# ==================================================================
# Phase 1 — WireGuard mgmt routes (kernel backend)
# ==================================================================
section "phase 1 — WireGuard control plane"

step "GET /api/protocol/wg/status — kernel backend, running, subnet seeded"
wg_status=$(req GET /api/protocol/wg/status)
echo "$wg_status" | jq . >&2
assert_eq "true"          "$(echo "$wg_status" | jq -r .running)"     "wg.running"
assert_eq "kernel"        "$(echo "$wg_status" | jq -r .backend)"     "wg.backend"
assert_eq "true"          "$(echo "$wg_status" | jq -r .available)"   "wg.available"
assert_eq "wg0"           "$(echo "$wg_status" | jq -r .interface)"   "wg.interface"
assert_eq "10.99.99.0/24" "$(echo "$wg_status" | jq -r .subnet)"      "wg.subnet"
assert_eq "51820"         "$(echo "$wg_status" | jq -r .listen_port)" "wg.listen_port"
SERVER_PUBKEY=$(echo "$wg_status" | jq -r .server_public_key)
[[ -n "$SERVER_PUBKEY" && "$SERVER_PUBKEY" != "null" ]] && pass "server_public_key present"
assert_eq "0" "$(echo "$wg_status" | jq -r .total_peers)" "wg.total_peers (initial)"

# ==================================================================
# Phase 2 — settings
# ==================================================================
section "phase 2 — settings"

step "GET /api/settings — initial state"
settings=$(req GET /api/settings)
assert_eq "10.99.99.0/24" "$(echo "$settings" | jq -r .wg_subnet)"      "settings.wg_subnet"
assert_eq "51820"         "$(echo "$settings" | jq -r .wg_listen_port)" "settings.wg_listen_port"
assert_eq "4433"          "$(echo "$settings" | jq -r .ss_listen_port)" "settings.ss_listen_port"

step "PATCH /api/settings — update domain"
patched=$(req PATCH /api/settings '{"domain":"e2e.example.com"}')
assert_eq "e2e.example.com" "$(echo "$patched" | jq -r .domain)" "settings.domain after patch"

step "PATCH /api/settings — change wg_subnet (allowed when no peers)"
patched=$(req PATCH /api/settings '{"wg_subnet":"10.88.88.0/24"}')
assert_eq "10.88.88.0/24" "$(echo "$patched" | jq -r .wg_subnet)" "wg_subnet flipped"
# Status view should reflect the new subnet.
assert_eq "10.88.88.0/24" "$(req GET /api/protocol/wg/status | jq -r .subnet)" "wg.subnet propagated"

step "PATCH /api/settings — restore wg_subnet"
patched=$(req PATCH /api/settings '{"wg_subnet":"10.99.99.0/24"}')
assert_eq "10.99.99.0/24" "$(echo "$patched" | jq -r .wg_subnet)" "wg_subnet restored"

step "POST /api/reload — apply settings"
code=$(req_status POST /api/reload)
assert_status "204" "$code" "reload"

step "PATCH /api/settings — deny_unknown_fields rejects garbage"
# axum's Json extractor surfaces serde deserialization errors as 422
# (Unprocessable Entity); a `deny_unknown_fields` violation lands
# there rather than at the explicit 400 path.
code=$(req_status PATCH /api/settings '{"bogus_field":42}')
if [[ "$code" == "400" || "$code" == "422" ]]; then
    pass "PATCH /api/settings unknown field rejected (HTTP $code)"
else
    fail "PATCH /api/settings unknown field: expected 400/422, got $code"
fi

step "GET /api/audit returns a JSON array"
audit=$(req "GET" "/api/audit?limit=10")
assert_eq "array" "$(echo "$audit" | jq -r 'type')" "audit response is an array"

# ==================================================================
# Phase 3 — Users + per-user WG (CRUD, idempotency, rotation, errors)
# ==================================================================
section "phase 3 — users + per-user WG"

step "POST /api/users — create alice"
alice=$(req POST /api/users '{"name":"alice","note":"e2e test user"}')
ALICE_ID=$(echo "$alice" | jq -r .id)
assert_eq "alice" "$(echo "$alice" | jq -r .name)" "alice.name"
assert_eq "false" "$(echo "$alice" | jq -r .wg_enabled)" "alice.wg_enabled (initial)"

step "POST /api/users — empty name rejected (400)"
code=$(req_status POST /api/users '{"name":""}')
assert_status "400" "$code" "empty name 400"

step "GET /api/users — alice listed"
listed_id=$(req GET /api/users | jq -r ".[] | select(.id==\"$ALICE_ID\") | .id")
assert_eq "$ALICE_ID" "$listed_id" "alice present in /api/users"

step "PATCH /api/users/:id — rename alice"
patched=$(req PATCH "/api/users/$ALICE_ID" '{"name":"alice2"}')
assert_eq "alice2" "$(echo "$patched" | jq -r .name)" "alice rename"

step "POST /api/users/:id/wg — enable WG for alice (server-generated keypair)"
enable=$(req POST "/api/users/$ALICE_ID/wg" '{}')
ALICE_PEER_ID=$(echo "$enable" | jq -r .peer.id)
ALICE_ALLOWED_IP=$(echo "$enable" | jq -r .peer.allowed_ip)
ALICE_PUBKEY=$(echo "$enable" | jq -r .peer.public_key)
[[ "$ALICE_ALLOWED_IP" =~ ^10\.99\.99\. ]] && pass "alice peer IP in subnet ($ALICE_ALLOWED_IP)" \
    || fail "alice peer ip $ALICE_ALLOWED_IP not in 10.99.99.0/24"
assert_eq "true" "$(echo "$enable" | jq -r .peer.has_psk)" "alice peer has psk"
[[ -n "$(echo "$enable" | jq -r .secrets.private_key)" ]] && pass "secrets.private_key returned once"

step "POST /api/users/:id/wg — second enable is idempotent (returns peer, no secrets)"
second=$(req POST "/api/users/$ALICE_ID/wg" '{}')
assert_eq "$ALICE_PEER_ID" "$(echo "$second" | jq -r .peer.id)" "idempotent peer.id"
assert_eq "$ALICE_ALLOWED_IP" "$(echo "$second" | jq -r .peer.allowed_ip)" "idempotent peer.allowed_ip"
priv_second=$(echo "$second" | jq -r '.secrets // empty')
assert_eq "" "$priv_second" "idempotent enable omits secrets"

wait_until 5 "wg.total_peers == 1" wg_total_peers_is 1

step "GET /api/users/:id/wg — peer detail (flat WgPeerDto)"
detail=$(req GET "/api/users/$ALICE_ID/wg")
assert_eq "$ALICE_PEER_ID" "$(echo "$detail" | jq -r .id)" "user wg detail id"

step "POST /api/users/:id/wg/rotate — rotate alice keypair, IP unchanged"
rotated=$(req POST "/api/users/$ALICE_ID/wg/rotate" '{}')
NEW_PUBKEY=$(echo "$rotated" | jq -r .peer.public_key)
NEW_IP=$(echo "$rotated" | jq -r .peer.allowed_ip)
assert_eq "$ALICE_ALLOWED_IP" "$NEW_IP" "rotated peer IP unchanged"
assert_ne "$ALICE_PUBKEY" "$NEW_PUBKEY" "rotated peer pubkey differs"
[[ -n "$(echo "$rotated" | jq -r .secrets.private_key)" ]] && pass "rotate emits new private key"
ALICE_PUBKEY=$NEW_PUBKEY

step "POST /api/users/:id/wg with caller-supplied pubkey — server stores verbatim, no private key returned"
BOB_PRIV=$(wg genkey)
BOB_PUB=$(echo "$BOB_PRIV" | wg pubkey)
bob=$(req POST /api/users '{"name":"bob"}')
BOB_ID=$(echo "$bob" | jq -r .id)
bob_enable=$(req POST "/api/users/$BOB_ID/wg" "{\"public_key\":\"$BOB_PUB\"}")
assert_eq "$BOB_PUB" "$(echo "$bob_enable" | jq -r .peer.public_key)" "bob server-stored pubkey matches caller's"
assert_eq "" "$(echo "$bob_enable" | jq -r '.secrets.private_key // empty')" "private_key omitted when caller supplied pubkey"
BOB_PEER_IP=$(echo "$bob_enable" | jq -r .peer.allowed_ip)
BOB_PEER_PSK=$(echo "$bob_enable" | jq -r '.secrets.preshared_key // empty')
[[ -n "$BOB_PEER_PSK" ]] && pass "preshared_key still returned"

step "POST /api/users/:id/wg — malformed pubkey rejected (400)"
code=$(req_status POST "/api/users/$BOB_ID/wg" '{"public_key":"not-base64-!!"}')
assert_status "400" "$code" "malformed pubkey 400"

step "POST /api/users/:id/wg — wrong-length pubkey rejected (400)"
code=$(req_status POST "/api/users/$BOB_ID/wg" '{"public_key":"AA=="}')
assert_status "400" "$code" "short pubkey 400"

wait_until 5 "wg.total_peers == 2" wg_total_peers_is 2

# ==================================================================
# Phase 4 — settings: SubnetConflict 409 path
# ==================================================================
section "phase 4 — settings: wg_subnet conflict"

step "PATCH /api/settings wg_subnet to a range that excludes existing peers (409)"
code=$(req_status PATCH /api/settings '{"wg_subnet":"172.16.99.0/24"}')
assert_status "409" "$code" "subnet change with existing peers"
detail=$(cat /tmp/last_body)
echo "$detail" | jq . >&2 || true
# Surface should be problem+json with the conflicting peer ids.
echo "$detail" | jq -e '.conflicts | length >= 2' >/dev/null && pass "conflict body lists 2+ peer ids" \
    || fail "conflict body missing .conflicts array (got: $detail)"
assert_eq "wg-subnet-conflict" "$(echo "$detail" | jq -r .code)" "conflict body code"

# ==================================================================
# Phase 5 — Shadowsocks lifecycle
# ==================================================================
section "phase 5 — Shadowsocks"

step "GET /api/protocol/ss/status — driver running"
ss_status=$(req GET /api/protocol/ss/status)
echo "$ss_status" | jq . >&2
assert_eq "true" "$(echo "$ss_status" | jq -r .running)" "ss.running"

step "POST /api/users/:id/ss — enable SS for alice"
ss_enable=$(req POST "/api/users/$ALICE_ID/ss" '')
echo "$ss_enable" | jq . >&2
ALICE_SS_PSK=$(echo "$ss_enable" | jq -r .psk)
ALICE_SS_URL=$(echo "$ss_enable" | jq -r .url)
[[ -n "$ALICE_SS_PSK" && "$ALICE_SS_PSK" != "null" ]] && pass "alice ss psk returned (hex)"
[[ "$ALICE_SS_URL" == ss://* ]] && pass "alice ss url is ss://… ($ALICE_SS_URL)" \
    || fail "alice ss url not ss://: $ALICE_SS_URL"

step "GET /api/users/:id/ss — public detail (no PSK)"
ss_detail=$(req GET "/api/users/$ALICE_ID/ss")
assert_eq "$ALICE_ID" "$(echo "$ss_detail" | jq -r .user_id)" "ss detail user_id"
assert_eq "" "$(echo "$ss_detail" | jq -r '.psk // empty')" "ss detail must not leak psk"

step "POST /api/users/:id/ss/rotate — fresh PSK"
rotated=$(req POST "/api/users/$ALICE_ID/ss/rotate")
NEW_SS_PSK=$(echo "$rotated" | jq -r .psk)
assert_ne "$ALICE_SS_PSK" "$NEW_SS_PSK" "ss psk rotated"

step "GET /api/users/:id/ss/qr — PNG content-type"
ct=$(curl -sS -o /tmp/qr.png -w '%{content_type}' \
        -H "Authorization: Bearer $TOKEN" \
        "${NSP_BASE}/api/users/$ALICE_ID/ss/qr")
assert_eq "image/png" "$ct" "qr content-type"
size=$(stat -c%s /tmp/qr.png)
[[ "$size" -gt 200 ]] && pass "qr png byte length > 200 ($size)" \
    || fail "qr png too small ($size)"

step "POST /api/protocol/ss/stop"
code=$(req_status POST /api/protocol/ss/stop)
assert_status "204" "$code" "ss stop"
wait_until 5 "ss.running == false" ss_running_is "false"

step "POST /api/protocol/ss/start"
code=$(req_status POST /api/protocol/ss/start)
assert_status "204" "$code" "ss start"
wait_until 5 "ss.running == true" ss_running_is "true"

step "DELETE /api/users/:id/ss — disable alice ss"
ack=$(req DELETE "/api/users/$ALICE_ID/ss")
assert_eq "false" "$(echo "$ack" | jq -r .pending)" "disable ss ack.pending=false"

# ==================================================================
# Phase 6 — iptables (driver baseline + user rule)
# ==================================================================
section "phase 6 — iptables"

step "GET /api/iptables — driver baseline rules registered"
rules=$(req GET /api/iptables)
echo "$rules" | jq '.[] | {source, table, chain, spec}' >&2
wg_rules=$(echo "$rules" | jq '[.[] | select(.source=="wg-driver")] | length')
[[ "$wg_rules" -ge 2 ]] && pass "wg-driver baseline rules present ($wg_rules)" \
    || fail "expected at least 2 wg-driver baseline rules, found $wg_rules"

step "POST /api/iptables/verify — well-formed user rule passes"
verify=$(req POST /api/iptables/verify '{"table":"filter","chain":"INPUT","spec":"-p tcp --dport 9999 -j ACCEPT"}')
assert_eq "true" "$(echo "$verify" | jq -r .ok)" "verify ok"

step "POST /api/iptables — register a user rule"
created=$(req POST /api/iptables '{"table":"filter","chain":"INPUT","spec":"-p tcp --dport 9999 -j ACCEPT","comment":"e2e"}')
RULE_ID=$(echo "$created" | jq -r .id)
assert_eq "user" "$(echo "$created" | jq -r .source)" "rule.source=user"

step "POST /api/iptables — shell metacharacters rejected (400)"
code=$(req_status POST /api/iptables '{"table":"filter","chain":"INPUT","spec":"-p tcp --dport 9999; rm -rf /"}')
assert_status "400" "$code" "shell injection rejected"

step "POST /api/iptables/reconcile — driver state matches kernel"
report=$(req POST /api/iptables/reconcile)
echo "$report" | jq . >&2
[[ "$(echo "$report" | jq -r .reinserted)" =~ ^[0-9]+$ ]] && pass "reconcile returns numeric report"

step "DELETE /api/iptables/:id — remove the user rule"
code=$(req_status DELETE "/api/iptables/$RULE_ID")
assert_status "204" "$code" "iptables delete"

step "GET /api/iptables?source=user — user rule gone"
remaining=$(req GET "/api/iptables?source=user")
[[ "$(echo "$remaining" | jq 'length')" == "0" ]] && pass "no user-source rules remain" \
    || fail "user rules still present: $remaining"

# ==================================================================
# Phase 7 — WG stop/start lifecycle, peers persist
# ==================================================================
section "phase 7 — WG lifecycle"

step "POST /api/protocol/wg/stop"
code=$(req_status POST /api/protocol/wg/stop)
assert_status "204" "$code" "wg stop transition"
wait_until 5 "wg.running == false" wg_running_is "false"

step "POST /api/protocol/wg/start"
code=$(req_status POST /api/protocol/wg/start)
assert_status "204" "$code" "wg start transition"
wait_until 5 "wg.running == true" wg_running_is "true"

# Peers must survive the stop/start cycle.
wait_until 10 "wg.total_peers stays 2 across stop/start" wg_total_peers_is 2

# ==================================================================
# Phase 8 — Data plane: real WG client → ping server
# ==================================================================
# Bring up a kernel WG interface inside this tester container and
# prove the kernel netlink path actually carries traffic. This is the
# strongest check we can do without a third container — encrypts
# traffic to nsp:51820, decrypts the reply, validates rx/tx_bytes
# increment in the API.
section "phase 8 — WG data plane"

# Bob already has a peer registered with our public key (BOB_PUB) and
# preshared key (BOB_PEER_PSK), and an allocated IP (BOB_PEER_IP).
# Use BOB_PRIV / BOB_PEER_IP to bring up our side.
WG_IF=wgtest0
DP_OK=1

setup_wg_client() {
    local server_ip
    server_ip=$(getent hosts "$NSP_SERVER_HOST" | awk '{print $1; exit}')
    [[ -n "$server_ip" ]] || die "could not resolve $NSP_SERVER_HOST"
    pass "server IP resolved: $server_ip"

    ip link del "$WG_IF" 2>/dev/null || true
    ip link add "$WG_IF" type wireguard
    ip address add "${BOB_PEER_IP}/32" dev "$WG_IF"

    local k_priv k_psk
    k_priv=$(mktemp); printf %s "$BOB_PRIV"      > "$k_priv"
    k_psk=$(mktemp);  printf %s "$BOB_PEER_PSK"  > "$k_psk"
    wg set "$WG_IF" \
        private-key "$k_priv" \
        peer "$SERVER_PUBKEY" \
        preshared-key "$k_psk" \
        endpoint "${server_ip}:51820" \
        allowed-ips 10.99.99.1/32 \
        persistent-keepalive 5
    rm -f "$k_priv" "$k_psk"

    # Bring the link up BEFORE adding the route — `ip route add` rejects
    # routes whose nexthop device is still down.
    ip link set "$WG_IF" up
    # Route ONLY the server's WG IP through the tunnel, so the API
    # connection (over the docker bridge) keeps working.
    ip route add 10.99.99.1/32 dev "$WG_IF"
    pass "kernel $WG_IF up, peer endpoint=${server_ip}:51820"
}

teardown_wg_client() {
    ip link del "$WG_IF" 2>/dev/null || true
}

step "bring up kernel WG client inside tester"
if setup_wg_client; then
    step "ping server's WG IP through the tunnel"
    if ping -c 3 -W 3 -i 0.3 10.99.99.1 >/tmp/ping.log 2>&1; then
        pass "ping 10.99.99.1 succeeded ($(grep -oE 'time=[0-9.]+ ms' /tmp/ping.log | head -1))"
    else
        cat /tmp/ping.log >&2
        fail "ping through tunnel failed"
        DP_OK=0
    fi

    if [[ "$DP_OK" == "1" ]]; then
        # Counters are read straight off the kernel via netlink — but
        # leave the API one polling window to read them.
        peer_field_gt() {
            # $1 = peer id, $2 = field name (rx_bytes|tx_bytes)
            local v
            v=$(req GET "/api/users/$1/wg" | jq -r ".$2")
            [[ "$v" =~ ^[0-9]+$ && "$v" -gt 0 ]]
        }
        wait_until 5 "bob peer rx_bytes > 0 in API" peer_field_gt "$BOB_ID" rx_bytes
        wait_until 5 "bob peer tx_bytes > 0 in API" peer_field_gt "$BOB_ID" tx_bytes

        # The wg-tools CLI inside the tester should also see a recent
        # handshake with the server.
        if wg show "$WG_IF" latest-handshakes | awk '{print $2}' | grep -qE '^[1-9]'; then
            pass "wg show reports a recent handshake on $WG_IF"
        else
            fail "wg show reports no handshake"
        fi
    fi

    teardown_wg_client
    pass "tunnel torn down"
else
    fail "could not bring up kernel WG client"
fi

# ==================================================================
# Phase 9 — Metrics endpoint
# ==================================================================
section "phase 9 — /metrics"

step "GET /metrics requires auth"
code=$(curl -sS -o /tmp/last_body -w '%{http_code}' --max-time 10 "${NSP_BASE}/metrics")
assert_status "401" "$code" "/metrics without bearer token"

step "GET /metrics with bearer token returns Prometheus text"
code=$(curl -sS -o /tmp/metrics -w '%{http_code}' --max-time 10 \
        -H "Authorization: Bearer $NSP_METRICS_TOKEN" \
        "${NSP_BASE}/metrics")
assert_status "200" "$code" "/metrics with bearer"
grep -q "^nsp_wg_peers" /tmp/metrics && pass "metric nsp_wg_peers present" \
    || fail "metric nsp_wg_peers missing"
grep -q "^nsp_http_requests_total" /tmp/metrics && pass "metric nsp_http_requests_total present" \
    || fail "metric nsp_http_requests_total missing"

# ==================================================================
# Phase 10 — auth: password rotation invalidates old JWT
# ==================================================================
section "phase 10 — auth rotation"

NEW_PASSWORD="rotated-$(openssl rand -hex 4)"
OLD_TOKEN=$TOKEN
step "PATCH /api/settings — change admin password"
patched=$(req PATCH /api/settings "{\"new_password\":\"$NEW_PASSWORD\"}")
new_tgen=$(echo "$patched" | jq -r .token_generation)
[[ "$new_tgen" -ge 1 ]] && pass "token_generation bumped (=$new_tgen)"

step "old JWT is now rejected (401)"
TOKEN=$OLD_TOKEN
code=$(req_status GET /api/me)
assert_status "401" "$code" "old token rejected after password change"

step "POST /api/auth/login with new password works"
relogin "$NEW_PASSWORD"
me=$(req GET /api/me)
assert_eq "admin" "$(echo "$me" | jq -r .sub)" "/api/me with new token"

step "Restore original password (cleanup)"
patched=$(req PATCH /api/settings "{\"new_password\":\"$NSP_ADMIN_PASSWORD\"}")
relogin "$NSP_ADMIN_PASSWORD"

# ==================================================================
# Phase 11 — cleanup + cascade delete
# ==================================================================
section "phase 11 — cleanup"

step "DELETE /api/users/:id/wg — disable bob"
ack=$(req DELETE "/api/users/$BOB_ID/wg")
assert_eq "false" "$(echo "$ack" | jq -r .pending)" "disable bob ack.pending=false"

wait_until 5 "wg.total_peers == 1 after disabling bob" wg_total_peers_is 1

step "DELETE /api/users/:id — delete bob (cascade clears any leftover state)"
code=$(req_status DELETE "/api/users/$BOB_ID")
assert_status "204" "$code" "delete bob"

step "DELETE /api/users/:id — delete alice2"
code=$(req_status DELETE "/api/users/$ALICE_ID")
assert_status "204" "$code" "delete alice"

# Reconciler eventually drops orphaned peers.
wait_until 15 "wg.total_peers == 0 (reconciler converged)" wg_total_peers_is 0

# ==================================================================
# Phase 12 — error-path hygiene
# ==================================================================
section "phase 12 — error paths"

step "GET /api/users/missing — 404"
code=$(req_status GET "/api/users/00000000-0000-0000-0000-000000000000")
assert_status "404" "$code" "404 on unknown user"

step "Unauthenticated request rejected (401)"
saved=$TOKEN; TOKEN=""
code=$(req_status GET /api/me)
assert_status "401" "$code" "/api/me requires auth"
TOKEN=$saved

# ==================================================================
# Summary
# ==================================================================
printf "\n${YEL}--- E2E SUMMARY ---${NC}\n" >&2
printf "ran:    %s\n" "$TESTS_RUN" >&2
printf "failed: %s\n" "$TESTS_FAILED" >&2
if [[ "$TESTS_FAILED" -gt 0 ]]; then
    printf "${RED}E2E FAILED${NC}\n" >&2
    exit 1
fi
printf "${GREEN}E2E PASSED${NC}\n" >&2
