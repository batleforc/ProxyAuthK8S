/** Trigger a browser download of `content` as a file named `filename`. */
export function downloadTextFile(
  content: string,
  filename: string,
  type = 'application/yaml',
) {
  const blob = new Blob([content], { type });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
