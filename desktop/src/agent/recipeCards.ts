export interface RecipeItem {
  id?: string;
  label?: string;
  count?: number;
}

export interface RecipeCardData {
  version: 1;
  type: string;
  title?: string;
  result?: RecipeItem;
  grid?: Array<Array<RecipeItem | null>>;
  ingredients?: RecipeItem[];
  source_chunk_ids?: string[];
}

export type RecipeTextSegment =
  | { type: "markdown"; text: string }
  | { type: "recipe_card"; card: RecipeCardData };

const RECIPE_FENCE = /```recipe_card\s*\n([\s\S]*?)\n```/g;

export function parseRecipeCardBlocks(text: string): RecipeTextSegment[] {
  const segments: RecipeTextSegment[] = [];
  let cursor = 0;
  let matched = false;

  for (const match of text.matchAll(RECIPE_FENCE)) {
    const start = match.index ?? 0;
    const raw = match[0];
    const json = match[1];
    const card = parseRecipeCard(json);
    if (!card) continue;
    matched = true;
    pushMarkdown(segments, text.slice(cursor, start));
    segments.push({ type: "recipe_card", card });
    cursor = start + raw.length;
  }

  if (!matched) return [{ type: "markdown", text }];
  pushMarkdown(segments, text.slice(cursor));
  return segments;
}

function pushMarkdown(segments: RecipeTextSegment[], text: string): void {
  const trimmed = trimBlankLines(text);
  if (trimmed) segments.push({ type: "markdown", text: trimmed });
}

function trimBlankLines(text: string): string {
  return text.replace(/^\s*\n/, "").replace(/\n\s*$/, "");
}

function parseRecipeCard(json: string): RecipeCardData | null {
  let value: unknown;
  try {
    value = JSON.parse(json);
  } catch {
    return null;
  }
  if (!value || typeof value !== "object") return null;
  const input = value as Record<string, unknown>;
  if (input.version !== 1 || typeof input.type !== "string" || !input.type.trim()) return null;

  const card: RecipeCardData = {
    version: 1,
    type: input.type.trim(),
  };
  if (typeof input.title === "string" && input.title.trim()) card.title = input.title.trim();
  const result = parseItem(input.result);
  if (result) card.result = result;
  const grid = parseGrid(input.grid);
  if (grid) card.grid = grid;
  const ingredients = parseItems(input.ingredients);
  if (ingredients.length > 0) card.ingredients = ingredients;
  const sources = parseStrings(input.source_chunk_ids);
  if (sources.length > 0) card.source_chunk_ids = sources;
  return card;
}

function parseGrid(value: unknown): Array<Array<RecipeItem | null>> | undefined {
  if (!Array.isArray(value)) return undefined;
  const rows = value
    .slice(0, 3)
    .map((row) =>
      Array.isArray(row)
        ? row.slice(0, 3).map((cell) => parseItem(cell))
        : [null, null, null],
    );
  if (rows.length === 0) return undefined;
  return rows.map((row) => {
    const out = row.slice(0, 3);
    while (out.length < 3) out.push(null);
    return out;
  });
}

function parseItems(value: unknown): RecipeItem[] {
  if (!Array.isArray(value)) return [];
  return value.map(parseItem).filter((item): item is RecipeItem => Boolean(item));
}

function parseItem(value: unknown): RecipeItem | null {
  if (value == null) return null;
  if (typeof value === "string") return { id: value, label: value };
  if (!value || typeof value !== "object") return null;
  const input = value as Record<string, unknown>;
  const id = typeof input.id === "string" ? input.id.trim() : "";
  const label = typeof input.label === "string" ? input.label.trim() : "";
  const count = typeof input.count === "number" && Number.isFinite(input.count) ? input.count : undefined;
  if (!id && !label) return null;
  return {
    ...(id ? { id } : {}),
    ...(label ? { label } : {}),
    ...(count && count > 0 ? { count } : {}),
  };
}

function parseStrings(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((v) => (typeof v === "string" ? v.trim() : ""))
    .filter(Boolean);
}
