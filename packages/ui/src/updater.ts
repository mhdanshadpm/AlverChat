/**
 * The install sequence behind Settings → About → Install, as one function so the order and the
 * refusal are testable without a signed artifact: the plugin verifies the signature while
 * `downloadAndInstall` runs, so a bad bundle must stop the sequence before the restart.
 */
export async function installAndRestart(
  update: { downloadAndInstall: () => Promise<void> },
  relaunch: () => Promise<void>,
): Promise<void> {
  await update.downloadAndInstall();
  await relaunch();
}
