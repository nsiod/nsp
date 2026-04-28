// English resource bundle for the nsp UI.
//
// Namespaces (keep this comment in sync when adding keys):
//   - common:     shared chrome (buttons, status words, pagination, errors)
//   - login:      LoginPage
//   - users:      UsersPage (list view, toggles, dialog)
//   - userDetail: UserDetailPage (single-user view + protocol cards)
//   - audit:      AuditPage
//   - settings:   SettingsPage
//   - services:   ServicesPage

export const en = {
  common: {
    appName: 'NSP',
    signOut: 'Sign out',
    loading: 'Loading…',
    cancel: 'Cancel',
    close: 'Close',
    retry: 'Retry',
    previousPage: 'Previous page',
    nextPage: 'Next page',
    pageXofY: 'page {{page}} of {{total}}',
    badgeOn: 'on',
    badgeOff: 'off',
    enabled: 'enabled',
    disabled: 'disabled',
    statusUnavailable: 'status unavailable',
    language: 'Language',
    languageEnglish: 'English',
    languageChinese: '中文',
    theme: {
      label: 'Theme',
      light: 'Light',
      dark: 'Dark',
      system: 'System',
    },
    nav: {
      users: 'Users',
      audit: 'Audit',
      settings: 'Settings',
      services: 'Services',
      iptables: 'Firewall',
    },
    protocolStrip: {
      shadowsocks: 'Shadowsocks',
      wireguard: 'WireGuard',
    },
    secretReveal: {
      defaultDescription: 'Copy or download these now. The server will not show them again.',
      done: 'I have saved a copy',
      copy: 'Copy',
      copyAria: 'Copy {{label}}',
      download: 'Download',
      downloadAria: 'Download {{label}}',
      showQr: 'Show QR',
      reloadQr: 'Reload QR',
      showQrAria: 'Show QR for {{label}}',
      qrAlt: 'QR for {{label}}',
      copied: 'Copied',
      copiedBody: '{{label}} copied to clipboard',
      copyFailed: 'Copy failed',
      qrFailed: 'QR failed',
      showAdvanced: 'Show raw keys',
      hideAdvanced: 'Hide raw keys',
    },
  },

  login: {
    title: 'Sign in to nsp',
    description: 'Enter the admin password.',
    passwordLabel: 'Password',
    submit: 'Sign in',
    submitting: 'Signing in…',
    failedTitle: 'Login failed',
  },

  users: {
    heading: 'Users',
    subtitle: 'Manage Shadowsocks and WireGuard access per user.',
    newUser: 'New user',
    searchPlaceholder: 'Search users…',
    searchAria: 'Search users',
    countOne: '{{count}} user',
    countOther: '{{count}} users',
    table: {
      name: 'Name',
      ss: 'SS',
      wg: 'WG',
      created: 'Created',
      actions: 'Actions',
    },
    empty: 'No users yet.',
    emptyFiltered: 'No matches.',
    detail: 'Detail',
    openAria: 'Open {{name}}',
    deleteAria: 'Delete {{name}}',
    toggleSsAria: 'Toggle Shadowsocks for {{name}}',
    toggleWgAria: 'Toggle WireGuard for {{name}}',
    confirmRemove: 'Remove {{name}} from all protocols?',
    driverWarningOne:
      '{{proto}} driver is unavailable on the server. You can still create users; toggles for that protocol are disabled until the backend reports a healthy status.',
    driverWarningMany:
      '{{protos}} drivers are unavailable on the server. You can still create users; toggles for those protocols are disabled until the backend reports a healthy status.',
    driverPaused:
      '{{protos}} driver is paused (preconditions not met). New and enabled accounts persist now; the server will sync them into the live driver as soon as it recovers.',
    driverJoiner: ' and ',
    toasts: {
      enableSsFailed: 'Enable SS failed',
      disableSsFailed: 'Disable SS failed',
      enableWgFailed: 'Enable WG failed',
      disableWgFailed: 'Disable WG failed',
      deleteFailed: 'Delete user failed',
      created: 'User created',
      protocolEnableFailed: 'User created, but protocol enablement failed',
      createFailed: 'Create failed',
    },
    reveal: {
      ssTitle: 'Shadowsocks credentials for {{name}}',
      wgTitle: 'WireGuard config for {{name}}',
      sip002Label: 'SIP002 URL',
      pskLabel: 'User pre-shared key (iPSK)',
      serverPskLabel: 'Server pre-shared key (uPSK)',
      wgConfLabel: 'wg-quick config',
      privateKeyLabel: 'Private key',
    },
    dialog: {
      title: 'New user',
      description: 'Names must be 1–32 characters, letters / digits / underscores / hyphens only.',
      nameLabel: 'Name',
      namePlaceholder: 'alice',
      noteLabel: 'Note (optional)',
      notePlaceholder: 'Free-form description',
      shadowsocks: 'Shadowsocks',
      wireguard: 'WireGuard',
      enableSsAria: 'Enable Shadowsocks for new user',
      enableWgAria: 'Enable WireGuard for new user',
      submit: 'Create user',
      submitting: 'Creating…',
    },
  },

  userDetail: {
    backToUsers: 'Back to users',
    subtitle:
      'Single identity across protocols. Rotating a key invalidates the previous client config.',
    notFoundTitle: 'User not found',
    notFoundDescription: 'Couldn’t locate “{{name}}” in the user registry.',
    shadowsocks: {
      title: 'Shadowsocks',
      subtitle: '2022-blake3-aes-256-gcm, single-port multi-user',
      enable: 'Enable Shadowsocks',
      rotate: 'Rotate PSK',
      confirmDisable: 'Disable Shadowsocks for this user?',
      confirmRotate: 'Rotate the Shadowsocks PSK? Existing clients will stop working.',
      revealCreated: 'Shadowsocks enabled — {{name}}',
      revealRotated: 'Shadowsocks rotated — {{name}}',
      meta: {
        userId: 'User ID',
        created: 'Created',
        sip002: 'SIP002 URL',
      },
    },
    wireguard: {
      title: 'WireGuard',
      subtitle: 'userspace device, single peer per user',
      enable: 'Enable WireGuard',
      rotate: 'Rotate keypair',
      confirmDisable: 'Disable WireGuard for this user?',
      confirmRotate: 'Rotate the WireGuard keypair? Existing clients must reload the new config.',
      revealCreated: 'WireGuard enabled — {{name}}',
      revealRotated: 'WireGuard rotated — {{name}}',
      meta: {
        peerId: 'Peer ID',
        allowedIp: 'Allowed IP',
        endpoint: 'Endpoint',
        publicKey: 'Public key',
        lastHandshake: 'Last handshake',
        traffic: 'Traffic',
      },
      importKey: {
        title: 'Use an existing public key',
        description:
          'Paste a base64-encoded WireGuard public key (32 bytes / 44 chars). The server will store it as-is and never see your private key.',
        placeholder: 'AbCdEf… (44 chars)',
        submit: 'Enable with this key',
        invalid: 'Public key must be 32 bytes (44 base64 characters).',
      },
    },
    notEnabled: 'Not enabled. Toggle on to issue credentials.',
    paused: 'Driver is paused. Enabling now persists the account; the server will activate it as soon as the driver recovers.',
    working: 'Working…',
    toggleAria: 'Toggle {{title}}',
    danger: {
      title: 'Danger zone',
      description: 'Removes this user from every protocol. Cannot be undone.',
      confirm: 'Delete {{name}} from all protocols?',
      button: 'Delete user',
    },
    toasts: {
      enableSsFailed: 'Enable SS failed',
      disableSsFailed: 'Disable SS failed',
      enableWgFailed: 'Enable WG failed',
      disableWgFailed: 'Disable WG failed',
      deleteFailed: 'Delete user failed',
      rotateFailed: 'Rotate failed',
    },
    reveal: {
      sip002: 'SIP002 URL',
      psk: 'User pre-shared key (iPSK)',
      serverPsk: 'Server pre-shared key (uPSK)',
      wgConf: 'wg-quick config',
      privateKey: 'Private key',
    },
  },

  audit: {
    heading: 'Audit log',
    description:
      'Append-only record of mutating API calls. Read access requires the <1>/api/audit</1> endpoint.',
    unavailableTitle: 'Audit log unavailable',
    unavailableBody:
      'The server could not return audit entries from <1>AuditRepo::append</1> through <3>GET /api/audit</3>. Check the API logs and retry.',
    filterPlaceholder: 'Filter by actor, action, target…',
    filterAria: 'Filter audit entries',
    entryCount: '{{count}} entries',
    table: {
      timestamp: 'Timestamp',
      actor: 'Actor',
      action: 'Action',
      target: 'Target',
      detail: 'Detail',
    },
    empty: 'No audit entries yet',
    emptyDescription: 'Mutating API calls will show up here as soon as one is made.',
    emptyFiltered: 'No entries match the filter',
  },

  settings: {
    heading: 'Settings',
    subtitle: 'Server-wide configuration. Changes apply to the running drivers without a restart.',
    reload: 'Reload',
    reloading: 'Reloading…',
    reloadHelp:
      'Re-assert the current DB state into the running SS/WG drivers. Useful after manual DB edits.',
    network: {
      title: 'Network',
      subtitle: 'Host and WireGuard pool.',
      domainLabel: 'Domain',
      domainPlaceholder: 'proxy.example.com',
      domainHelp: 'Used for ACME enrolment and as the domain portion of generated client configs.',
      subnetLabel: 'WireGuard subnet',
      subnetPlaceholder: '10.255.0.0/16',
      subnetHelp:
        'Blank means no server-managed pool (clients must supply their own IP). Changing the subnet re-allocates peer IPs; existing clients will need new configs.',
      ssPortLabel: 'Shadowsocks port',
      ssPortHelp: 'The SS listener rebinds hot; active connections on the old port are dropped.',
      wgPortLabel: 'WireGuard listen port',
      wgPortHelp: 'Requires a driver restart to take effect (set in boot config today).',
    },
    credentials: {
      title: 'Admin credentials',
      description:
        'Update the password used for <1>/api/auth/login</1>. All active sessions will be invalidated.',
      newPassword: 'New password',
      confirmPassword: 'Confirm new password',
    },
    save: 'Save changes',
    savingProgress: 'Saving…',
    status: {
      heading: 'Diagnostics',
      description: 'Read-only telemetry from the live settings row.',
      tokenGeneration: 'JWT token generation',
      tokenGenerationHelp:
        'Increments every time the admin password is rotated. Existing JWTs from older generations are rejected, forcing every signed-in operator to log in again.',
      updatedAt: 'Last settings update',
      updatedAtHelp: 'Server-side timestamp of the latest write to this settings row.',
    },
    toasts: {
      passwordMismatchTitle: 'Passwords differ',
      passwordMismatchBody: 'Confirm the new admin password and try again.',
      saved: 'Settings updated',
      updateFailed: 'Update failed',
      subnetConflictTitle: 'Subnet conflict',
      subnetConflictBody:
        '{{count}} peer(s) fall outside the requested subnet: {{ids}}. Resolve them before retrying.',
      credentialsRotatedTitle: 'Sign in again',
      credentialsRotatedBody:
        'Admin credentials were rotated — every session was signed out, including this one.',
      reloaded: 'Reload dispatched',
      reloadFailed: 'Reload failed',
    },
  },

  services: {
    heading: 'Services',
    subtitle:
      'Start and stop data-plane services. Runtime state is independent of the boot config.',
    running: 'running',
    stopped: 'stopped',
    loading: 'Loading status…',
    start: 'Start',
    stop: 'Stop',
    startAria: 'Start {{name}}',
    stopAria: 'Stop {{name}}',
    startFailed: 'Start {{name}} failed',
    stopFailed: 'Stop {{name}} failed',
    driverDown:
      'Driver not initialised on the server. Enable it in the boot config to manage at runtime.',
    preconditions: 'Preconditions not met:',
    descriptions: {
      shadowsocks: 'Embedded AEAD-2022 server. Runtime lifecycle is independent of config.',
      wireguard: 'Userspace WireGuard device. Requires CAP_NET_ADMIN and /dev/net/tun.',
    },
    metrics: {
      users: 'Users',
      reloads: 'Reloads',
      lastSwap: 'Last swap',
      peers: 'Peers',
      listenPort: 'Listen port',
      endpoint: 'Endpoint',
    },
    detail: 'Details',
    detailAria: 'Open {{name}} details',
  },

  serviceDetail: {
    back: 'Back to services',
    notFoundTitle: 'Service not found',
    notFoundDescription: 'Unknown service identifier in the URL.',
    unavailable: 'preconditions not met',
    statusHeading: 'Runtime',
    usersHeading_one: '{{count}} user',
    usersHeading_other: '{{count}} users',
    usersDescription: 'Accounts currently enabled on this service.',
    usersEmpty: 'No users have this service enabled yet.',
    ss: {
      title: 'Shadowsocks',
      publicHost: 'Public host',
      listenPort: 'Listen port',
      method: 'Cipher',
      users: 'Active users',
      reloads: 'Reloads',
      lastSwap: 'Last swap',
    },
    wg: {
      title: 'WireGuard',
      interface: 'Interface',
      listenPort: 'Listen port',
      subnet: 'Subnet',
      endpoint: 'Endpoint host',
      serverPublicKey: 'Server public key',
      totalPeers: 'Total peers',
    },
  },

  iptables: {
    heading: 'Firewall rules',
    subtitle:
      'User-defined firewall rules and baseline rules installed by drivers. Only user rules can be deleted from the UI.',
    newRule: 'New rule',
    newRuleAria: 'Create new firewall rule',
    reconcile: 'Reconcile',
    reconciling: 'Reconciling…',
    reconcileAria: 'Re-sync live firewall state with stored rules',
    empty: 'No firewall rules yet',
    emptyDescription:
      'Add a custom rule to control traffic into or out of the proxy. Driver-managed rules will appear here automatically once their service starts.',
    driverDown:
      'Firewall backend is not available on the server. Ensure the `iptables` binary is installed and reachable.',
    confirmDelete: 'Delete rule from {{table}}/{{chain}}?',
    deleteAria: 'Delete rule {{id}}',
    filter: {
      all: 'All',
      user: 'User',
      wgDriver: 'WireGuard driver',
    },
    sources: {
      user: 'user',
      wgDriver: 'wg-driver',
    },
    table: {
      heading: 'Managed rules',
      description:
        'Rules tagged with `nsp:<source>:<uuid>`. Driver rules are re-applied on service start.',
      source: 'Source',
      chain: 'Table / Chain',
      spec: 'Spec',
      priority: 'Priority',
      comment: 'Comment',
      created: 'Created',
      actions: 'Actions',
    },
    dialog: {
      title: 'New firewall rule',
      description:
        'Rules are written via iptables with a `nsp:user:<uuid>` comment tag and persisted for reconcile.',
      tableLabel: 'Table',
      chainLabel: 'Chain',
      specLabel: 'Spec',
      specPlaceholder: '-s 10.0.0.0/8 -j ACCEPT',
      specHelp:
        'Arguments appended after the chain. Do not include `-t` or `-A` — those are added automatically.',
      priorityLabel: 'Priority',
      commentLabel: 'Comment',
      commentPlaceholder: 'Optional human-readable note',
      submit: 'Create rule',
      submitting: 'Creating…',
    },
    sshGuard: {
      title: 'SSH guard tripped',
      description:
        'The rule you submitted would interfere with SSH (tcp/22). Confirm to retry with force=true.',
      confirm: 'I understand — force apply',
      confirming: 'Applying…',
    },
    toasts: {
      created: 'Rule created',
      createFailed: 'Create rule failed',
      deleted: 'Rule deleted',
      deleteFailed: 'Delete rule failed',
      reconciled: 'Reconcile complete',
      reconciledBody: 'Re-inserted {{reinserted}}, pruned {{pruned}}, kept {{kept}}.',
      reconcileFailed: 'Reconcile failed',
    },
  },
};

export type EnResources = typeof en;

export default en;
