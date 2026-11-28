/** Put text on the clipboard; false when the system refused (nothing is thrown). */
export async function copyText(text: string): Promise<boolean> {
  try {
    // Missing in some web views and test DOMs.
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    if (!navigator.clipboard) return false;
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
