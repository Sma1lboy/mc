// Confirm the clipboard write before announcing success. Keep the command visible
// for browsers where clipboard access is unavailable or denied.
const copyButton = document.getElementById("copy-brew");
const copyStatus = document.getElementById("copy-status");
copyButton.addEventListener("click", async () => {
  if (copyButton.disabled) return;
  copyButton.disabled = true;
  copyStatus.textContent = "正在复制…";
  try {
    await navigator.clipboard.writeText(document.getElementById("brew-cmd").textContent);
    copyStatus.textContent = "已复制安装命令";
  } catch {
    copyStatus.textContent = "未能复制。请选中上方命令,手动复制。";
  } finally {
    copyButton.disabled = false;
  }
});
