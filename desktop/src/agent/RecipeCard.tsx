import { useEffect, useMemo, useState } from "react";
import { commands } from "../ipc/bindings";
import { activeRoot } from "../store";
import { useChatStore } from "./chatStore";
import type { RecipeCardData, RecipeItem } from "./recipeCards";

type SpectaResult<T> = { status: "ok"; data: T } | { status: "error"; error: string };

type IconMap = Record<string, string>;

export function RecipeCard({ card }: { card: RecipeCardData }) {
  const wiki = useChatStore((s) => s.toolContext?.wiki ?? null);
  const ids = useMemo(() => collectResolvableItemIds(card), [card]);
  const [icons, setIcons] = useState<IconMap>({});
  const root = wiki?.root || activeRoot();
  const instanceId = wiki?.instanceId || "";

  useEffect(() => {
    if (!instanceId || ids.length === 0) {
      setIcons({});
      return;
    }
    let cancelled = false;
    void Promise.all(
      ids.map(async (id) => {
        const icon = await unwrap(commands.resolveItemIcon(root, instanceId, id)).catch(() => null);
        return icon ? ([id, icon.data_url] as const) : null;
      }),
    ).then((entries) => {
      if (cancelled) return;
      setIcons(Object.fromEntries(entries.filter((entry): entry is readonly [string, string] => Boolean(entry))));
    });
    return () => {
      cancelled = true;
    };
  }, [ids, instanceId, root]);

  const title = card.title || card.result?.label || card.result?.id || "Recipe";

  return (
    <div className="my-[10px] max-w-full overflow-x-auto">
      <div className="inline-flex min-w-[320px] flex-col gap-[9px] bg-panel-2 shadow-sunken px-[12px] py-[10px]">
        <div className="flex items-start justify-between gap-[12px]">
          <div className="min-w-0">
            <div className="text-[13px] font-semibold leading-[1.3] text-fg break-words">{title}</div>
            <div className="mt-[2px] text-[10px] uppercase tracking-[0.08em] text-faint">
              {recipeKindLabel(card.type)}
            </div>
          </div>
          {card.result && <RecipeSlot item={card.result} icon={iconFor(icons, card.result)} result />}
        </div>

        {card.grid && card.grid.length > 0 ? (
          <div className="flex items-center gap-[10px]">
            <div className="grid grid-cols-3 gap-[3px]">
              {normalizeGrid(card.grid).map((item, i) => (
                <RecipeSlot key={i} item={item} icon={iconFor(icons, item)} />
              ))}
            </div>
            {card.result && (
              <>
                <div className="text-[18px] leading-none text-muted">-&gt;</div>
                <RecipeSlot item={card.result} icon={iconFor(icons, card.result)} result />
              </>
            )}
          </div>
        ) : (
          <div className="flex flex-wrap gap-[4px]">
            {(card.ingredients ?? []).map((item, i) => (
              <RecipeSlot key={`${item.id ?? item.label ?? "item"}-${i}`} item={item} icon={iconFor(icons, item)} />
            ))}
          </div>
        )}

        {card.source_chunk_ids && card.source_chunk_ids.length > 0 && (
          <div className="border-t border-titlebar pt-[6px] text-[10px] leading-[1.4] text-faint">
            {card.source_chunk_ids.join(" · ")}
          </div>
        )}
      </div>
    </div>
  );
}

function RecipeSlot({
  item,
  icon,
  result,
}: {
  item?: RecipeItem | null;
  icon?: string;
  result?: boolean;
}) {
  const label = item?.label || item?.id || "";
  return (
    <div
      title={label}
      className={`relative flex shrink-0 items-center justify-center overflow-hidden bg-panel shadow-input ${
        result ? "h-[48px] w-[48px]" : "h-[42px] w-[42px]"
      }`}
    >
      {item ? (
        icon ? (
          <img
            src={icon}
            alt={label}
            className="h-[30px] w-[30px] object-contain [image-rendering:pixelated]"
            draggable={false}
          />
        ) : (
          <span className="px-[3px] text-center text-[9px] font-medium leading-[1.1] text-sub break-words">
            {shortLabel(label)}
          </span>
        )
      ) : null}
      {item?.count && item.count > 1 && (
        <span className="absolute bottom-[2px] right-[3px] text-[10px] font-semibold leading-none text-fg drop-shadow">
          {item.count}
        </span>
      )}
    </div>
  );
}

function collectResolvableItemIds(card: RecipeCardData): string[] {
  const items: Array<RecipeItem | null | undefined> = [card.result];
  for (const row of card.grid ?? []) items.push(...row);
  items.push(...(card.ingredients ?? []));
  return Array.from(
    new Set(
      items
        .map((item) => item?.id?.trim() ?? "")
        .filter((id) => id.includes(":") && !id.startsWith("#")),
    ),
  );
}

function normalizeGrid(grid: Array<Array<RecipeItem | null>>): Array<RecipeItem | null> {
  const out: Array<RecipeItem | null> = [];
  for (let y = 0; y < 3; y++) {
    const row = grid[y] ?? [];
    for (let x = 0; x < 3; x++) out.push(row[x] ?? null);
  }
  return out;
}

function iconFor(icons: IconMap, item?: RecipeItem | null): string | undefined {
  return item?.id ? icons[item.id] : undefined;
}

function shortLabel(label: string): string {
  return label.replace(/^#/, "").replace(/_/g, " ").slice(0, 18);
}

function recipeKindLabel(kind: string): string {
  if (kind === "crafting_shaped") return "Crafting table";
  if (kind === "crafting_shapeless") return "Shapeless crafting";
  return kind.replace(/_/g, " ");
}

async function unwrap<T>(p: Promise<SpectaResult<T>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}
