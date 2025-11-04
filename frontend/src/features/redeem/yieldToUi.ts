export async function yieldToUi(): Promise<void> {
  await new Promise<void>((resolve) => {
    if (typeof window !== 'undefined') {
      if (typeof window.requestAnimationFrame === 'function') {
        window.requestAnimationFrame(() => resolve());
        return;
      }
      window.setTimeout(resolve, 0);
      return;
    }
    setTimeout(resolve, 0);
  });
}
