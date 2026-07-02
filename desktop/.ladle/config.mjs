/** @type {import('@ladle/react').UserConfig} */
export default {
  stories: "src/**/*.stories.{ts,tsx}",
  // Ladle 画布默认白底;这里压成和 app 外壳一致的暖黑,避免深色令牌组件飘在白底上。
  appendToHead: `<style>
    :root { color-scheme: dark; }
    body { background-color: #16170f; }
  </style>`,
  addons: {
    // 默认深色主题(与 app 默认一致);仍可在工具栏切浅色对照。
    theme: { enabled: true, defaultState: "dark" },
  },
};
