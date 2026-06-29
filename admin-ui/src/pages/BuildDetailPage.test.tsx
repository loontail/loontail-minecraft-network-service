import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes } from "react-router-dom";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { BuildDetailPage } from "@/pages/BuildDetailPage";
import type { VersionsCatalog } from "@/features/builds/useVersions";
import type { ClientAdmin, Keyword, Server } from "@/shared/types";
import { renderWithProviders } from "@/test/renderWithProviders";

// Radix Select drives its trigger/listbox through Pointer Events and scrolls the
// active item into view. jsdom implements neither, so polyfill the handful of
// methods Radix touches; without them, opening the listbox throws.
beforeAll(() => {
  if (!Element.prototype.hasPointerCapture) {
    Element.prototype.hasPointerCapture = () => false;
  }
  if (!Element.prototype.setPointerCapture) {
    Element.prototype.setPointerCapture = () => {};
  }
  if (!Element.prototype.releasePointerCapture) {
    Element.prototype.releasePointerCapture = () => {};
  }
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = () => {};
  }
});

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
function renderDetail() {
  return renderWithProviders(
    <Routes>
      <Route path="/builds/:slug" element={<BuildDetailPage />} />
    </Routes>,
    { route: "/builds/all-the-mods-9" },
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

const SAMPLE_BUILD: ClientAdmin = {
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

// Route the admin clients list (build resolution), the linked bundle read (manifest
// panel + always-mounted Files tab), the per-client media list (always-mounted
// Media tab), and the public keyword/server surfaces the Servers & Tags tab pulls
// from. The media branch must be checked before the generic clients branch since
// its URL also contains `/admin/catalog/clients`.
function mockApi({
  clients = [SAMPLE_BUILD] as ClientAdmin[],
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
    const user = userEvent.setup();
    renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );

    await user.click(screen.getByRole("tab", { name: /servers & tags/i }));

    // The attached keyword chip and the server row both render.
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

    // Removing the attached keyword chip is wired (detach control present).
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
    const user = userEvent.setup();
    await renderDetailsTab();

    // The Radix trigger is a combobox showing the build's stored MC value.
    const mc = await screen.findByRole("combobox", {
      name: "Minecraft version",
    });
    expect(mc).toHaveTextContent("1.21.4");

    // Opening it surfaces the catalog options as listbox items.
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
    const user = userEvent.setup();
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
    const user = userEvent.setup();
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
    const user = userEvent.setup();
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
    const user = userEvent.setup();
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
    const user = userEvent.setup();
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
    const user = userEvent.setup();
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
    const user = userEvent.setup();
    await renderDetailsTab();

    const java = await screen.findByRole("combobox", { name: "Java version" });
    await user.click(java);
    const listbox = await screen.findByRole("listbox");
    await user.click(within(listbox).getByRole("option", { name: /custom/i }));

    // The combobox is replaced by a text input the admin can type into.
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
    const user = userEvent.setup();
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
    const user = userEvent.setup();
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

describe("BuildDetailPage — tab switching preserves edits (P1-4)", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("keeps in-progress Details edits across a tab switch and back", async () => {
    mockApi();
    const user = userEvent.setup();
    renderDetail();

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "All the Mods 9" }),
      ).toBeInTheDocument(),
    );

    // Type a new title in the Details form.
    const title = screen.getByLabelText("Title") as HTMLInputElement;
    await user.clear(title);
    await user.type(title, "Edited Title");
    expect(title.value).toBe("Edited Title");

    // Switch away to another tab, then back to Details.
    await user.click(screen.getByRole("tab", { name: /servers & tags/i }));
    await user.click(screen.getByRole("tab", { name: /details/i }));

    // The in-progress edit survives because the panels stay mounted.
    expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe(
      "Edited Title",
    );
  });
});
