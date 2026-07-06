import { useEffect, useMemo, useState } from "react";
import { commands } from "../ipc/bindings";
import { activeRoot } from "../store";
import { useChatStore } from "./chatStore";
import {
  recipeCardIconIdsFromKey,
  recipeCardIconIdsKey,
  recipeItemIconLookupId,
  type RecipeCardData,
  type RecipeItem,
} from "./recipeCards";
import { uniqueSiblingKeys } from "./renderKeys";

type SpectaResult<T> = { status: "ok"; data: T } | { status: "error"; error: string };

type IconMap = Record<string, string>;

export function RecipeCard({ card }: { card: RecipeCardData }) {
  const wiki = useChatStore((s) => s.toolContext?.wiki ?? null);
  const iconIdsKey = useMemo(() => recipeCardIconIdsKey(card), [card]);
  const ids = useMemo(() => recipeCardIconIdsFromKey(iconIdsKey), [iconIdsKey]);
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
  }, [iconIdsKey, instanceId, root]);

  const title = card.title || card.result?.label || card.result?.id || "Recipe";
  const grid = card.grid && card.grid.length > 0 ? normalizeGrid(card.grid) : [];
  const gridKeys = uniqueSiblingKeys(grid, (item, i) => `slot:${i}:${item?.id ?? item?.label ?? "empty"}`);
  const ingredientKeys = uniqueSiblingKeys(
    card.ingredients ?? [],
    (item) => `ingredient:${item.id ?? item.label ?? "item"}`,
  );

  return (
    <div className="my-[10px] max-w-full overflow-x-auto">
      <div className="mc-recipe-card">
        <div className="mc-recipe-title">
          <span className="mc-recipe-name">{title}</span>
          <span className="mc-recipe-kind">{recipeKindLabel(card.type)}</span>
        </div>

        {grid.length > 0 ? (
          <div className="mc-recipe-workbench">
            <div className="mc-recipe-grid" aria-label="Crafting grid">
              {grid.map((item, i) => (
                <RecipeSlot key={gridKeys[i]} item={item} icon={iconFor(icons, item)} />
              ))}
            </div>
            {card.result && (
              <>
                <div className="mc-recipe-arrow" aria-hidden="true">
                  <span />
                </div>
                <RecipeSlot item={card.result} icon={iconFor(icons, card.result)} result />
              </>
            )}
          </div>
        ) : (
          <div className="mc-recipe-ingredients">
            {(card.ingredients ?? []).map((item, i) => (
              <RecipeSlot key={ingredientKeys[i]} item={item} icon={iconFor(icons, item)} />
            ))}
            {card.result && <RecipeSlot item={card.result} icon={iconFor(icons, card.result)} result />}
          </div>
        )}

        {card.source_chunk_ids && card.source_chunk_ids.length > 0 && (
          <div className="mc-recipe-source">
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
      className={result ? "mc-recipe-slot mc-recipe-slot-result" : "mc-recipe-slot"}
    >
      {item ? (
        icon ? (
          <img
            src={icon}
            alt={label}
            className="mc-recipe-icon"
            draggable={false}
          />
        ) : (
          <span className="mc-recipe-fallback">
            {shortLabel(label)}
          </span>
        )
      ) : null}
      {item?.count && item.count > 1 && (
        <span className="mc-recipe-count">
          {item.count}
        </span>
      )}
    </div>
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
  const id = recipeItemIconLookupId(item);
  return id ? icons[id] : undefined;
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
