// 流式 markdown 的分块工具(纯函数,无依赖)。
// ------------------------------------------------------------------
// splitBlocks:按空行把 markdown 源文切成块,但绝不在 ``` 围栏内部切分
// (逐行扫描时跟踪围栏开合状态)。已完成的块其源串在流式期间不再变化,
// 因此 UI 逐块渲染时只有最后一个(仍在增长的)块需要每帧重解析。
// hardenStreamingTail:仅用于渲染最后一个块的「渲染时」加固——半流式的
// 围栏补上闭合、孤悬的 **/` 剪掉,绝不改写存储的缓冲。
// ------------------------------------------------------------------

const FENCE = /^\s*```/;
const BLANK = /^\s*$/;

/** 按空行分块(围栏内不切)。返回的块串起来(以空行相隔)语义等价于原文。 */
export function splitBlocks(src: string): string[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const blocks: string[] = [];
  let cur: string[] = [];
  let inFence = false;
  for (const line of lines) {
    if (FENCE.test(line)) inFence = !inFence;
    if (!inFence && BLANK.test(line)) {
      if (cur.length) blocks.push(cur.join("\n"));
      cur = [];
    } else {
      cur.push(line);
    }
  }
  if (cur.length) blocks.push(cur.join("\n"));
  return blocks;
}

/** 统计非重叠出现次数。 */
function count(s: string, needle: string): number {
  let n = 0;
  let i = s.indexOf(needle);
  while (i !== -1) {
    n++;
    i = s.indexOf(needle, i + needle.length);
  }
  return n;
}

/**
 * 渲染时加固半流式块尾:
 *  - 围栏行数为奇数 → 补一个闭合 ```(半截代码块渲染成代码,而不是吞掉后续排版);
 *  - 否则若恰以孤悬的 **(总数为奇)或单个 `(总数为奇)结尾 → 剪掉该尾巴。
 */
export function hardenStreamingTail(block: string): string {
  const fences = block.split("\n").filter((l) => FENCE.test(l)).length;
  if (fences % 2 === 1) return `${block}\n\`\`\``;
  if (block.endsWith("**") && count(block, "**") % 2 === 1) return block.slice(0, -2);
  if (block.endsWith("`") && count(block, "`") % 2 === 1) return block.slice(0, -1);
  return block;
}
