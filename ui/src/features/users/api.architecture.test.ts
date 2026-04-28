import { describe, expect, it } from 'vitest';
import * as usersApi from './api';

describe('users feature api', () => {
  it('keeps user lifecycle mutations on the central /api/users surface', () => {
    // Per-user hooks live under /api/users; protocol hooks are
    // service-only (status/start/stop) and live in services/api.ts.
    expect(usersApi).toHaveProperty('useCreateUserMutation');
    expect(usersApi).toHaveProperty('useDeleteUserMutation');
    expect(usersApi).toHaveProperty('useEnableUserSsMutation');
    expect(usersApi).toHaveProperty('useDisableUserSsMutation');
    expect(usersApi).toHaveProperty('useRotateUserSsMutation');
    expect(usersApi).toHaveProperty('useUserSsDetailQuery');
    expect(usersApi).toHaveProperty('useEnableUserWgMutation');
    expect(usersApi).toHaveProperty('useDisableUserWgMutation');
    expect(usersApi).toHaveProperty('useRotateUserWgMutation');
    expect(usersApi).toHaveProperty('useUserWgDetailQuery');

    // None of the legacy protocol-user hooks may come back — they tempt
    // callers to reach around /api/users.
    expect(usersApi).not.toHaveProperty('useCreateSsUser');
    expect(usersApi).not.toHaveProperty('useDeleteSsUser');
    expect(usersApi).not.toHaveProperty('useCreateWgPeer');
    expect(usersApi).not.toHaveProperty('useDeleteWgPeer');
    expect(usersApi).not.toHaveProperty('useSsUsers');
    expect(usersApi).not.toHaveProperty('useSsUser');
    expect(usersApi).not.toHaveProperty('useSsUserConfig');
    expect(usersApi).not.toHaveProperty('useRotateSsUser');
    expect(usersApi).not.toHaveProperty('useWgPeers');
    expect(usersApi).not.toHaveProperty('useWgPeer');
    expect(usersApi).not.toHaveProperty('useRotateWgPeer');
  });
});
