import { describe, expect, it, vi } from 'vitest';
import { installAndRestart } from './updater';

describe('the updater install sequence', () => {
  it('restarts only after the bundle is installed', async () => {
    const order: string[] = [];
    await installAndRestart(
      {
        downloadAndInstall: async () => {
          order.push('install');
        },
      },
      async () => {
        order.push('relaunch');
      },
    );
    expect(order).toEqual(['install', 'relaunch']);
  });

  it('leaves the running app alone when the bundle is refused', async () => {
    const relaunch = vi.fn();
    await expect(
      installAndRestart(
        {
          downloadAndInstall: async () => {
            throw new Error('invalid signature');
          },
        },
        relaunch,
      ),
    ).rejects.toThrow('invalid signature');
    expect(relaunch).not.toHaveBeenCalled();
  });
});
