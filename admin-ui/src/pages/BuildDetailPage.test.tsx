import { act, screen, waitFor, within } from "@testing-library/react";
import { Route, Routes } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { buildKeys } from "@/features/builds/api";
import { BuildDetailPage } from "@/pages/BuildDetailPage";
import type { VersionsCatalog } from "@/features/builds/useVersions";
import type { BuildAdmin, Keyword, Server } from "@/shared/types";
import {
  makeTestQueryClient,
  renderWithProviders,
  setupUser,
} from "@/test/renderWithProviders";

const SAMPLE_VERSIONS: VersionsCatalog = {
  version: 3,
  minecraft: [
    { id: "1.21.4", type: "release" },
    { id: "1.20.1", type: "release" },
  ],
  fabric: ["0.16.10", "0.16.9"],
  forge: {
    "1.21.4": ["54.1.6", "54.1.5"],
    "1.20.1": ["47.4.0", "47.3.0"],
  },
  java: [
    { component: "java-runtime-delta", label: "Java 21 — java-runtime-delta", major: 21 },
    { component: "java-runtime-gamma", label: "Java 17 — java-runtime-gamma", major: 17 },
  ],
  recommended: {
    "1.21.4": {
      java: "java-runtime-delta",
      forge: "54.1.6",
      fabric: "0.16.10",
    },
    "1.20.1": {
      java: "java-runtime-gamma",
      forge: "47.4.0",
      fabric: "0.16.10",
    },
  },
  generatedAt: "2026-06-27T00:00:00.000Z",
};

// BuildDetailPage reads its `:slug` route param, so it must render inside a matching
// <Route> rather than bare.
function renderDetail(client = makeTestQueryClient()) {
  return renderWithProviders(
    <Routes>
      <Route path="/builds/:slug" element={<BuildDetailPage />} />
    </Routes>,
    { route: "/builds/all-the-mods-9", client },
  );
}

const SAMPLE_KEYWORD: Keyword = {
  id: "22222222222222222222222222222222",
  title: "Tech",
};

const SAMPLE_SERVER: Server = {
  id: "33333333333333333333333333333333",
  name: "Survival",
  address: "play.example.net",
};

const SAMPLE_BUILD: BuildAdmin = {
  id: "11111111111111111111111111111111",
  slug: "all-the-mods-9",
  title: "All the Mods 9",
  description: "",
  shortDescription: "",
  available: true,
  minecraftVersion: "1.21.4",
  forgeVersion: null,
  fabricVersion: null,
  runtimeVersion: "java-runtime-delta",
  bundleSlug: "all-the-mods-9",
  background: null,
  poster: null,
  titleImage: null,
  screenshots: [],
  keywords: [SAMPLE_KEYWORD],
  servers: [SAMPLE_SERVER],
  bundle: {
    slug: "all-the-mods-9",
    version: "1.0.0",
    status: "ready",
    filesCount: 7,
    manifestUrl: "/api/bundle-registry/builds/all-the-mods-9/manifest",
  },
  published: true,
};

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function errorResponse(status: number, message: string): Response {
  return new Response(JSON.stringify({ error: { code: "internal", message } }), {
    status,
    headers: { "content-type": "application/json" },
  });
}

// Route the admin builds list (build resolution), the linked bundle read (manifest
// panel + always-mounted Files tab), the per-build media list (always-mounted
// Media tab), and the public keyword/server surfaces the Servers & Tags tab pulls
// from. The media branch must be checked before the generic clients branch since
// its URL also contains `/admin/catalog/clients`.
function mockApi({
  clients = [SAMPLE_BUILD] as BuildAdmin[],
  keywords = [] as Keyword[],
  servers = [] as Server[],
  versions = SAMPLE_VERSIONS as VersionsCatalog,
} = {}) {
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("versions.json")) {
        return Promise.resolve(jsonResponse(versions));
      }
      if (url.includes("/media")) {
        return Promise.resolve(jsonResponse({ media: [] }));
      }
      if (url.includes("/admin/catalog/clients")) {
        return Promise.resolve(jsonResponse({ clients }));
      }
      if (url.includes("/admin/bundles/builds/")) {
        return Promise.resolve(
          jsonResponse({ ...SAMPLE_BUILD.bundle, artifacts: [] }),
        );
      }
      if (url.includes("/api/keywords")) {
        return Promise.resolve(jsonResponse({ keywords }));
      }
      if (url.includes("/api/servers")) {
        return Promise.resolve(jsonResponse({ servers }));
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`));
    }),
  );
}

describe("BuildDetailPage — Servers & Tags", () => {
  beforeEach(() => {
    mockApi();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("exposes a Servers & Tags tab", async () => {
    renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("tab", { name: /servers & tags/i }),
    ).toBeInTheDocument();
  });

  it("renders attached keywords and servers with add controls", async () => {
    const user = setupUser();
    renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );

    await user.click(screen.getByRole("tab", { name: /servers & tags/i }));

    expect(screen.getByText("Tech")).toBeInTheDocument();
    expect(screen.getByText("Survival")).toBeInTheDocument();
    expect(screen.getByText("play.example.net")).toBeInTheDocument();

    // Add controls exist for each section — Radix Select triggers expose
    // role="combobox" labelled by their aria-label.
    expect(
      screen.getByRole("combobox", { name: /add keyword/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("combobox", { name: /add server/i }),
    ).toBeInTheDocument();

    const chip = screen.getByText("Tech").closest("span");
    expect(chip).not.toBeNull();
    expect(
      within(chip as HTMLElement).getByRole("button", { name: /remove tech/i }),
    ).toBeInTheDocument();
  });
});

describe("BuildDetailPage — version dropdowns", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function renderDetailsTab() {
    renderDetail();
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );
  }

  it("renders the Minecraft version select with the stored value and catalog options", async () => {
    mockApi();
    const user = setupUser();
    await renderDetailsTab();

    // The Radix trigger is a combobox showing the build's stored MC value.
    const mc = await screen.findByRole("combobox", {
      name: "Minecraft version",
    });
    expect(mc).toHaveTextContent("1.21.4");

    await user.click(mc);
    const listbox = await screen.findByRole("listbox");
    expect(
      within(listbox).getByRole("option", { name: "1.21.4" }),
    ).toBeInTheDocument();
    expect(
      within(listbox).getByRole("option", { name: "1.20.1" }),
    ).toBeInTheDocument();
  });

  it("enables Forge once a Minecraft version is chosen, filtered to it", async () => {
    mockApi();
    const user = setupUser();
    await renderDetailsTab();

    // The sample build already has MC 1.21.4, so Forge is enabled and scoped.
    const forge = await screen.findByRole("combobox", { name: "Forge version" });
    await waitFor(() => expect(forge).not.toBeDisabled());

    await user.click(forge);
    const listbox = await screen.findByRole("listbox");
    expect(
      within(listbox).getByRole("option", { name: "54.1.6" }),
    ).toBeInTheDocument();
    // A forge build belonging to a different MC must NOT appear.
    expect(
      within(listbox).queryByRole("option", { name: "47.4.0" }),
    ).not.toBeInTheDocument();
  });

  it("disables Forge and Fabric until a Minecraft version is set", async () => {
    mockApi({ clients: [{ ...SAMPLE_BUILD, minecraftVersion: null }] });
    await renderDetailsTab();

    const forge = await screen.findByRole("combobox", { name: "Forge version" });
    const fabric = await screen.findByRole("combobox", {
      name: "Fabric version",
    });
    expect(forge).toBeDisabled();
    expect(fabric).toBeDisabled();
  });

  it("preserves a legacy Minecraft value not in the catalog", async () => {
    mockApi({ clients: [{ ...SAMPLE_BUILD, minecraftVersion: "1.7.10" }] });
    const user = setupUser();
    await renderDetailsTab();

    const mc = await screen.findByRole("combobox", {
      name: "Minecraft version",
    });
    // The out-of-list value stays selected (shown on the trigger)...
    expect(mc).toHaveTextContent("1.7.10");

    // ...and is injected into the option list so editing never drops it.
    await user.click(mc);
    const listbox = await screen.findByRole("listbox");
    expect(
      within(listbox).getByRole("option", { name: "1.7.10" }),
    ).toBeInTheDocument();
  });

  it("renders a Java select with the stored component and catalog components plus Custom…", async () => {
    mockApi();
    const user = setupUser();
    await renderDetailsTab();

    const java = await screen.findByRole("combobox", { name: "Java version" });
    // The trigger shows the stored component's label (no "recommended" badge text).
    expect(java).toHaveTextContent("java-runtime-delta");
    expect(java).not.toHaveTextContent(/recommended/i);

    await user.click(java);
    const listbox = await screen.findByRole("listbox");
    expect(
      within(listbox).getByRole("option", { name: /java-runtime-delta/ }),
    ).toBeInTheDocument();
    expect(
      within(listbox).getByRole("option", { name: /java-runtime-gamma/ }),
    ).toBeInTheDocument();
    expect(
      within(listbox).getByRole("option", { name: /custom/i }),
    ).toBeInTheDocument();
  });

  it("preserves a legacy/out-of-list Java runtime value", async () => {
    // An old build saved a bare major ("25") — surface it so it shows + can be cleared.
    mockApi({ clients: [{ ...SAMPLE_BUILD, runtimeVersion: "25" }] });
    const user = setupUser();
    await renderDetailsTab();

    const java = await screen.findByRole("combobox", { name: "Java version" });
    expect(java).toHaveTextContent("25");

    await user.click(java);
    const listbox = await screen.findByRole("listbox");
    expect(
      within(listbox).getByRole("option", { name: /^25/ }),
    ).toBeInTheDocument();
  });

  it("marks the recommended option in the dependent dropdowns", async () => {
    mockApi();
    const user = setupUser();
    await renderDetailsTab();

    // Forge: the recommended 54.1.6 carries the marker; its sibling does not.
    const forge = await screen.findByRole("combobox", { name: "Forge version" });
    await waitFor(() => expect(forge).not.toBeDisabled());
    await user.click(forge);
    let listbox = await screen.findByRole("listbox");
    const forgeRec = within(listbox).getByRole("option", { name: /54\.1\.6/ });
    expect(
      within(forgeRec).getByLabelText("recommended"),
    ).toBeInTheDocument();
    const forgeOther = within(listbox).getByRole("option", {
      name: /54\.1\.5/,
    });
    expect(
      within(forgeOther).queryByLabelText("recommended"),
    ).not.toBeInTheDocument();
    await user.keyboard("{Escape}");

    // Java: java-runtime-delta carries the marker, java-runtime-gamma does not.
    const java = await screen.findByRole("combobox", { name: "Java version" });
    await user.click(java);
    listbox = await screen.findByRole("listbox");
    const javaRec = within(listbox).getByRole("option", {
      name: /java-runtime-delta/,
    });
    expect(within(javaRec).getByLabelText("recommended")).toBeInTheDocument();
    const javaOther = within(listbox).getByRole("option", {
      name: /java-runtime-gamma/,
    });
    expect(
      within(javaOther).queryByLabelText("recommended"),
    ).not.toBeInTheDocument();
  });

  it("keeps the trigger value clean after selecting a recommended option", async () => {
    mockApi();
    const user = setupUser();
    await renderDetailsTab();

    const fabric = await screen.findByRole("combobox", {
      name: "Fabric version",
    });
    await waitFor(() => expect(fabric).not.toBeDisabled());
    await user.click(fabric);
    const listbox = await screen.findByRole("listbox");
    await user.click(
      within(listbox).getByRole("option", { name: /0\.16\.10/ }),
    );

    // The trigger must show the bare value, never the badge text (typeahead-safe).
    expect(fabric).toHaveTextContent("0.16.10");
    expect(fabric).not.toHaveTextContent(/recommended/i);
  });

  it("swaps the Java select for a free-text input when Custom… is picked", async () => {
    mockApi();
    const user = setupUser();
    await renderDetailsTab();

    const java = await screen.findByRole("combobox", { name: "Java version" });
    await user.click(java);
    const listbox = await screen.findByRole("listbox");
    await user.click(within(listbox).getByRole("option", { name: /custom/i }));

    const input = await screen.findByRole("textbox", { name: "Java version" });
    await user.clear(input);
    await user.type(input, "27");
    expect((input as HTMLInputElement).value).toBe("27");
    expect(
      screen.queryByRole("combobox", { name: "Java version" }),
    ).not.toBeInTheDocument();
  });

  it("fills recommended Forge/Java when the Minecraft version changes", async () => {
    // Start with no Forge/Java picks so the cascade seeds the recommended ones.
    mockApi({
      clients: [
        {
          ...SAMPLE_BUILD,
          minecraftVersion: "1.21.4",
          forgeVersion: null,
          fabricVersion: null,
          runtimeVersion: "",
        },
      ],
    });
    const user = setupUser();
    await renderDetailsTab();

    const mc = await screen.findByRole("combobox", {
      name: "Minecraft version",
    });
    await user.click(mc);
    let listbox = await screen.findByRole("listbox");
    await user.click(within(listbox).getByRole("option", { name: "1.20.1" }));

    // Forge -> recommended for 1.20.1, Java -> recommended component for 1.20.1.
    await waitFor(() =>
      expect(
        screen.getByRole("combobox", { name: "Forge version" }),
      ).toHaveTextContent("47.4.0"),
    );
    expect(
      screen.getByRole("combobox", { name: "Java version" }),
    ).toHaveTextContent("java-runtime-gamma");

    // Forge listbox surfaces the new MC's options, not the old MC's.
    const forge = screen.getByRole("combobox", { name: "Forge version" });
    await user.click(forge);
    listbox = await screen.findByRole("listbox");
    expect(
      within(listbox).getByRole("option", { name: /47\.4\.0/ }),
    ).toBeInTheDocument();
    expect(
      within(listbox).queryByRole("option", { name: /54\.1\.6/ }),
    ).not.toBeInTheDocument();
  });

  it("falls back to java[0] when the chosen MC has no recommended.java", async () => {
    // 1.20.1 catalog entry with recommended but NO java -> seed from java[0]'s component.
    const versions: VersionsCatalog = {
      ...SAMPLE_VERSIONS,
      recommended: {
        ...SAMPLE_VERSIONS.recommended,
        "1.20.1": { forge: "47.4.0", fabric: "0.16.10" },
      },
    };
    mockApi({
      versions,
      clients: [
        {
          ...SAMPLE_BUILD,
          minecraftVersion: "1.21.4",
          forgeVersion: null,
          fabricVersion: null,
          runtimeVersion: "",
        },
      ],
    });
    const user = setupUser();
    await renderDetailsTab();

    const mc = await screen.findByRole("combobox", {
      name: "Minecraft version",
    });
    await user.click(mc);
    const listbox = await screen.findByRole("listbox");
    await user.click(within(listbox).getByRole("option", { name: "1.20.1" }));

    // No recommended.java for 1.20.1 -> Java seeds from catalog.java[0].component.
    await waitFor(() =>
      expect(
        screen.getByRole("combobox", { name: "Java version" }),
      ).toHaveTextContent("java-runtime-delta"),
    );
  });
});

// A fetch stub whose admin-clients answer is read from a mutable holder (so a
// refetch can return different rows) and can be held pending on demand.
function mockApiWithLiveClients(initial: BuildAdmin[]) {
  const state = {
    clients: initial,
    holdNext: false,
    failClients: false,
    release: undefined as undefined | (() => void),
  };
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("versions.json")) {
        return Promise.resolve(jsonResponse(SAMPLE_VERSIONS));
      }
      if (url.includes("/media")) {
        return Promise.resolve(jsonResponse({ media: [] }));
      }
      if (url.includes("/admin/catalog/clients")) {
        const answer = () =>
          state.failClients
            ? errorResponse(500, "database is unavailable")
            : jsonResponse({ clients: state.clients });
        if (state.holdNext) {
          state.holdNext = false;
          return new Promise<Response>((resolve) => {
            state.release = () => resolve(answer());
          });
        }
        return Promise.resolve(answer());
      }
      if (url.includes("/admin/bundles/builds/")) {
        return Promise.resolve(
          jsonResponse({ ...SAMPLE_BUILD.bundle, artifacts: [] }),
        );
      }
      if (url.includes("/api/keywords")) {
        return Promise.resolve(jsonResponse({ keywords: [] }));
      }
      if (url.includes("/api/servers")) {
        return Promise.resolve(jsonResponse({ servers: [] }));
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`));
    }),
  );
  return state;
}

describe("BuildDetailPage — manifest URL", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders the manifest URL the server shipped, resolved against the origin", async () => {
    mockApi();
    renderDetail();

    const input = await screen.findByDisplayValue(
      `${window.location.origin}/api/bundle-registry/builds/all-the-mods-9/manifest`,
    );
    expect(input).toBeInTheDocument();
  });

  it("uses the server value even when it diverges from the slug-derived guess", async () => {
    mockApi({
      clients: [
        {
          ...SAMPLE_BUILD,
          bundle: {
            ...SAMPLE_BUILD.bundle!,
            manifestUrl: "/registry/v2/builds/other-slug/manifest.json",
          },
        },
      ],
    });
    renderDetail();

    expect(
      await screen.findByDisplayValue(
        `${window.location.origin}/registry/v2/builds/other-slug/manifest.json`,
      ),
    ).toBeInTheDocument();
  });
});

describe("BuildDetailPage — just-created build", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("shows the skeleton, not 'Build not found', while the stale list refetches", async () => {
    // The state after a create: BuildsPage already populated the admin list, the
    // invalidated refetch that will contain the new build is still in flight.
    const state = mockApiWithLiveClients([SAMPLE_BUILD]);
    const client = makeTestQueryClient();
    client.setQueryData(buildKeys.list(), [] as BuildAdmin[]);
    state.holdNext = true;

    renderDetail(client);

    await waitFor(() => expect(state.release).toBeDefined());
    expect(screen.queryByText(/Build not found/i)).not.toBeInTheDocument();

    await act(async () => {
      state.release?.();
    });

    expect(
      await screen.findByRole("heading", { name: "All the Mods 9" }),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Build not found/i)).not.toBeInTheDocument();
  });

  it("still reports a genuinely missing build once the list is idle", async () => {
    mockApiWithLiveClients([]);
    renderDetail();

    expect(await screen.findByText(/Build not found/i)).toBeInTheDocument();
  });

  it("blames the failed list load, not the build, when the backend is down", async () => {
    const state = mockApiWithLiveClients([SAMPLE_BUILD]);
    state.failClients = true;
    const user = setupUser();
    renderDetail();

    expect(await screen.findByText(/couldn.t load builds/i)).toBeInTheDocument();
    expect(screen.getByText(/database is unavailable/i)).toBeInTheDocument();
    expect(screen.queryByText(/Build not found/i)).not.toBeInTheDocument();

    // Retry re-reads the list, so recovery does not need a page reload.
    state.failClients = false;
    await user.click(screen.getByRole("button", { name: /retry/i }));

    expect(
      await screen.findByRole("heading", { name: "All the Mods 9" }),
    ).toBeInTheDocument();
  });
});

describe("BuildDetailPage — unsaved edits vs background refetch", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("keeps unsaved Title edits when the server copy changes underneath", async () => {
    const state = mockApiWithLiveClients([SAMPLE_BUILD]);
    const user = setupUser();
    const { client } = renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );

    const title = screen.getByLabelText("Title") as HTMLInputElement;
    await user.clear(title);
    await user.type(title, "My unsaved edit");

    // Another admin's edit lands via any invalidation of the clients list.
    state.clients = [{ ...SAMPLE_BUILD, title: "Renamed elsewhere" }];
    await act(async () => {
      await client.invalidateQueries({ queryKey: buildKeys.list() });
    });

    // The refetch really landed (the heading reflects the server copy)...
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Renamed elsewhere" }),
      ).toBeInTheDocument(),
    );
    // ...but the in-progress edit is not clobbered.
    expect(
      (screen.getByLabelText("Title") as HTMLInputElement).value,
    ).toBe("My unsaved edit");
  });

  it("follows the server value again once a no-op edit is undone", async () => {
    const state = mockApiWithLiveClients([SAMPLE_BUILD]);
    const user = setupUser();
    const { client } = renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );

    // A no-op edit sequence: the switch goes off and straight back on, so the form
    // is byte-identical to the server again.
    const available = screen.getByRole("switch", { name: "Available" });
    await user.click(available);
    await user.click(available);
    expect(available).toBeChecked();

    // Another admin changes a field this admin never touched.
    state.clients = [
      { ...SAMPLE_BUILD, title: "Renamed elsewhere", minecraftVersion: "1.20.1" },
    ];
    await act(async () => {
      await client.invalidateQueries({ queryKey: buildKeys.list() });
    });

    // The pristine form adopts the newest server values instead of pinning the
    // snapshot it held when the first toggle fired.
    await waitFor(() =>
      expect(
        (screen.getByLabelText("Title") as HTMLInputElement).value,
      ).toBe("Renamed elsewhere"),
    );
    expect(
      screen.getByRole("combobox", { name: "Minecraft version" }),
    ).toHaveTextContent("1.20.1");
  });

  it("re-seeds a field the user reverted, keeping the newest server value", async () => {
    const state = mockApiWithLiveClients([SAMPLE_BUILD]);
    const user = setupUser();
    const { client } = renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );

    // Type an edit, then undo it by hand (the same state a failed save leaves
    // behind once the admin gives up).
    const title = screen.getByLabelText("Title") as HTMLInputElement;
    await user.clear(title);
    await user.type(title, "Half-typed");
    await user.clear(title);
    await user.type(title, "All the Mods 9");

    state.clients = [{ ...SAMPLE_BUILD, title: "Renamed elsewhere" }];
    await act(async () => {
      await client.invalidateQueries({ queryKey: buildKeys.list() });
    });

    await waitFor(() =>
      expect(
        (screen.getByLabelText("Title") as HTMLInputElement).value,
      ).toBe("Renamed elsewhere"),
    );
  });

  it("follows the server value while the form is untouched", async () => {
    const state = mockApiWithLiveClients([SAMPLE_BUILD]);
    const { client } = renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );

    state.clients = [{ ...SAMPLE_BUILD, title: "Renamed elsewhere" }];
    await act(async () => {
      await client.invalidateQueries({ queryKey: buildKeys.list() });
    });

    await waitFor(() =>
      expect(
        (screen.getByLabelText("Title") as HTMLInputElement).value,
      ).toBe("Renamed elsewhere"),
    );
  });
});

describe("BuildDetailPage — tab switching preserves edits", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("keeps in-progress Details edits across a tab switch and back", async () => {
    mockApi();
    const user = setupUser();
    renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );

    const title = screen.getByLabelText("Title") as HTMLInputElement;
    await user.clear(title);
    await user.type(title, "Edited Title");
    expect(title.value).toBe("Edited Title");

    await user.click(screen.getByRole("tab", { name: /servers & tags/i }));
    await user.click(screen.getByRole("tab", { name: /details/i }));

    // The in-progress edit survives because the panels stay mounted.
    expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe(
      "Edited Title",
    );
  });
});
