import { useState } from "react";
import type { Story, StoryDefault } from "@ladle/react";
import { FacetSidebar, type FacetSelection } from "./FacetSidebar";
import type { FacetTagsDto } from "../ipc/types";

/* ============================================================================
 * facetSidebar.stories —— Discover 筛选侧栏的隔离预览(Ladle)。
 *
 * FacetSidebar 是纯 props:喂一份 mock 分类法(FacetTagsDto)+ 受控 selection,
 * onChange 由本地 useState 兜。不碰后端。
 * ========================================================================== */

export default {
  title: "Components / FacetSidebar",
} satisfies StoryDefault;

const MOCK_TAGS: FacetTagsDto = {
  categories: [
    { name: "adventure", header: "categories", project_type: "modpack" },
    { name: "technology", header: "categories", project_type: "modpack" },
    { name: "magic", header: "categories", project_type: "modpack" },
    { name: "optimization", header: "categories", project_type: "modpack" },
    { name: "multiplayer", header: "categories", project_type: "modpack" },
    { name: "kitchen-sink", header: "categories", project_type: "modpack" },
  ],
  loaders: [
    { name: "fabric", supported_project_types: ["modpack", "mod"] },
    { name: "forge", supported_project_types: ["modpack", "mod"] },
    { name: "neoforge", supported_project_types: ["modpack", "mod"] },
    { name: "quilt", supported_project_types: ["modpack", "mod"] },
  ],
  game_versions: [
    { version: "1.21.1", version_type: "release" },
    { version: "1.20.1", version_type: "release" },
    { version: "1.19.2", version_type: "release" },
    { version: "1.18.2", version_type: "release" },
  ],
};

const EMPTY: FacetSelection = {
  categories: [],
  loaders: [],
  gameVersions: [],
  environment: null,
  openSource: false,
};

export const Empty: Story = () => {
  const [sel, setSel] = useState<FacetSelection>(EMPTY);
  return (
    <div className="max-w-[260px]">
      <FacetSidebar kind="modpack" provider="modrinth" selected={sel} onChange={setSel} tags={MOCK_TAGS} />
    </div>
  );
};
Empty.storyName = "FacetSidebar · 未选(全部项)";

export const WithSelection: Story = () => {
  const [sel, setSel] = useState<FacetSelection>({
    categories: ["technology", "optimization"],
    loaders: ["fabric"],
    gameVersions: ["1.20.1"],
    environment: null,
    openSource: true,
  });
  return (
    <div className="max-w-[260px]">
      <FacetSidebar kind="modpack" provider="modrinth" selected={sel} onChange={setSel} tags={MOCK_TAGS} />
    </div>
  );
};
WithSelection.storyName = "FacetSidebar · 已选若干筛选";

export const Loading: Story = () => (
  <div className="max-w-[260px]">
    <FacetSidebar kind="modpack" provider="modrinth" selected={EMPTY} onChange={() => {}} tags={undefined} />
  </div>
);
Loading.storyName = "FacetSidebar · 分类法未就绪(loading)";
