// "tags" 命名空间词条:实例标签(自由格式)+ 库页按标签筛选。zh 真相源;en 回退中文。
const dict = {
  zh: {
    // 库页筛选条
    filterAll: "全部",
    filterLabel: "标签",
    // 实例详情页标签编辑器
    sectionTitle: "标签",
    addPlaceholder: "添加标签…",
    add: "添加",
    remove: "移除标签 {{ tag }}",
    empty: "暂无标签。添加标签以便在库里分组与筛选。",
    saveError: "保存标签失败:{{ err }}",
  },
  en: {
    filterAll: "All",
    filterLabel: "Tags",
    sectionTitle: "Tags",
    addPlaceholder: "Add a tag…",
    add: "Add",
    remove: "Remove tag {{ tag }}",
    empty: "No tags yet. Add tags to group and filter in the Library.",
    saveError: "Failed to save tags: {{ err }}",
  } as Record<string, string>,
};

export default dict;
