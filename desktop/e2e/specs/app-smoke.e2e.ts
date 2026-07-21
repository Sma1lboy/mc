type ThemeConfig = {
  mode: "dark" | "light" | "system";
  hue: number;
  saturation: number;
  lightness: number;
};

type PrimaryPage = "home" | "discover" | "library" | "agent" | "settings";

async function mockExternalBoundaries(): Promise<void> {
  const values: Array<[string, unknown]> = [
    ["kobemc_session", null],
    ["kobe_list_credentials", []],
    ["modrinth_search", []],
    ["content_facets", { categories: [], loaders: [], game_versions: [] }],
    ["list_versions", []],
    ["detect_java", []],
  ];
  for (const [command, value] of values) {
    const mock = await browser.tauri.mock(command);
    await mock.mockResolvedValue(value);
  }
}

async function bootApp(): Promise<void> {
  await mockExternalBoundaries();
  await browser.execute(async () => {
    const e2eWindow = window as Window & { __MC_E2E_START__?: () => Promise<void> };
    if (typeof e2eWindow.__MC_E2E_START__ !== "function") {
      throw new Error("E2E bootstrap is unavailable");
    }
    await e2eWindow.__MC_E2E_START__();
  });
  await expect($("[data-testid='app-shell']")).toBeDisplayed();
}

async function openPage(page: PrimaryPage): Promise<void> {
  const navTestId = `nav-${page}`;
  const pageTestId = `page-${page}`;
  console.info(`[E2E click] ${navTestId} -> ${pageTestId}`);

  const nav = await $(`[data-testid="${navTestId}"]`);
  await nav.click();
  await expect(nav).toHaveAttribute("aria-current", "page");
  await expect($(`[data-testid="${pageTestId}"]`)).toBeDisplayed();
}

describe("kobeMC desktop smoke", () => {
  it("starts the real app and navigates every primary page", async () => {
    await bootApp();
    await expect($("[data-testid='primary-nav']")).toBeDisplayed();

    await openPage("discover");
    await openPage("library");
    await openPage("agent");
    await openPage("settings");
    await openPage("home");
  });

  it("round-trips a real Rust IPC value and persists it across an app restart", async () => {
    const expected: ThemeConfig = {
      mode: "light",
      hue: 214,
      saturation: 63,
      lightness: 52,
    };
    await browser.tauri.execute(({ core }, value) => core.invoke("set_theme", { cfg: value }), expected);
    const beforeRestart = (await browser.tauri.execute(({ core }) =>
      core.invoke("get_theme"),
    )) as ThemeConfig;
    expect(beforeRestart).toEqual(expected);

    await browser.reloadSession();
    await bootApp();
    const afterRestart = (await browser.tauri.execute(({ core }) =>
      core.invoke("get_theme"),
    )) as ThemeConfig;
    expect(afterRestart).toEqual(expected);
  });
});
