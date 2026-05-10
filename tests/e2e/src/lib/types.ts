// Subset of the nsp HTTP API DTOs the e2e suite reads. Kept loose
// (mostly optional / `unknown` for nested fields) so a server-side
// schema add doesn't break the suite — only fields the assertions
// actually check are typed.

export interface Healthz {
  ok: boolean;
}

export interface Status {
  version: string;
  wg_enabled: boolean;
  ss_enabled: boolean;
}

export interface WgStatus {
  running: boolean;
  available: boolean;
  backend: string;
  interface: string;
  subnet: string | null;
  listen_port: number;
  server_public_key: string;
  total_peers: number;
}

export interface Settings {
  domain: string | null;
  wg_subnet: string | null;
  wg_listen_port: number;
  ss_listen_port: number;
  token_generation: number;
}

export interface User {
  id: string;
  name: string;
  note: string | null;
  wg_enabled: boolean;
  source?: "local" | "control" | string;
}

export interface WgPeerDto {
  id: string;
  user_id?: string | null;
  public_key: string;
  allowed_ip: string;
  has_psk: boolean;
  rx_bytes: number;
  tx_bytes: number;
}

export interface WgEnableResponse {
  peer: WgPeerDto;
  secrets?: {
    private_key?: string;
    preshared_key?: string;
  };
}

export interface SsStatus {
  running: boolean;
}

export interface SsEnableResponse {
  psk: string;
  url: string;
}

export interface SsDetail {
  user_id: string;
  psk?: string;
}

export interface IptablesRule {
  id: string;
  source: "wg-driver" | "user" | string;
  table: string;
  chain: string;
  spec: string;
}

export interface IptablesVerify {
  ok: boolean;
}

export interface ReconcileReport {
  reinserted: number;
}

export interface SubnetConflict {
  code: string;
  conflicts: string[];
}

export interface DisableAck {
  pending: boolean;
}

export interface AuthLogin {
  token: string;
}

export interface Me {
  sub: string;
}
