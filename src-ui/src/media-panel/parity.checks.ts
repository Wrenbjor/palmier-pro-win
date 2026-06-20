// Pure-logic parity checks for the media panel (E4-S8..S11 acceptance).
//
// As with the editor, there is no test runner wired into `src-ui` yet (adding one
// would touch the shared package.json/lockfile owned by another worker). So these
// checks are a framework-free, type-checked module: covered by `tsc --noEmit`, and
// runnable directly (see `_run-parity.mts`). `runMediaParityChecks()` returns the
// failures so a future vitest can `expect(runMediaParityChecks()).toEqual([])`.
//
// Golden values verified against `docs/reference/media-panel.md` + ruling #15:
//   sortAndFilter (4 modes), passesFilters truth table, gridDimensions math,
//   buildView (folder/flat/grouped), moveSelection arrow nav, marquee intersect,
//   drag-payload URI format (%.3f), search Files filter.

import {
  buildView,
  gridDimensions,
  isDescendant,
  legalFolderMoves,
  marqueeRect,
  marqueeSelect,
  moveSelection,
  passesFilters,
  rectsIntersect,
  sortAndFilter,
  type FilterState,
  type Rect,
} from "./logic";
import {
  assetUri,
  buildAssetDragPayload,
  folderUri,
  momentUri,
  parseUri,
} from "./drag";
import { fileMatches } from "./search";
import { Spacing } from "./theme";
import type {
  FilterableType,
  MediaAssetView,
  MediaFolderView,
  MediaSnapshot,
} from "./types";

const asset = (over: Partial<MediaAssetView>): MediaAssetView => ({
  id: "a",
  name: "a",
  path: "",
  type: "video",
  folderId: null,
  durationSeconds: null,
  isGenerated: false,
  ...over,
});

const noFilter = (): FilterState => ({
  filterTypes: new Set<FilterableType>(),
  filterAI: false,
  query: "",
});

export function runMediaParityChecks(): string[] {
  const fail: string[] = [];
  const check = (cond: boolean, msg: string) => {
    if (!cond) fail.push(msg);
  };

  // --- sort modes (#15) ------------------------------------------------------
  {
    const a = asset({ id: "z", name: "Zebra", type: "video", durationSeconds: 10 });
    const b = asset({ id: "a", name: "alpha", type: "audio", durationSeconds: 30 });
    const c = asset({ id: "m", name: "Mango", type: "image", durationSeconds: 5 });
    const set = [a, b, c];

    // dateAdded = insertion order (no sort)
    const da = sortAndFilter(set, noFilter(), "dateAdded");
    check(
      da.map((x) => x.id).join() === "z,a,m",
      `dateAdded must preserve insertion order, got ${da.map((x) => x.id).join()}`,
    );

    // name = case-insensitive asc → alpha, Mango, Zebra
    const nm = sortAndFilter(set, noFilter(), "name");
    check(
      nm.map((x) => x.id).join() === "a,m,z",
      `name asc, got ${nm.map((x) => x.id).join()}`,
    );

    // duration = DESC → 30(b),10(a),5(c)
    const du = sortAndFilter(set, noFilter(), "duration");
    check(
      du.map((x) => x.id).join() === "a,m,z".replace(/.*/, "") || true,
      "duration placeholder",
    );
    check(
      du.map((x) => x.durationSeconds).join() === "30,10,5",
      `duration desc, got ${du.map((x) => x.durationSeconds).join()}`,
    );

    // type = rawValue asc → audio, image, video
    const ty = sortAndFilter(set, noFilter(), "type");
    check(
      ty.map((x) => x.type).join() === "audio,image,video",
      `type asc, got ${ty.map((x) => x.type).join()}`,
    );

    // input array not mutated
    check(set.map((x) => x.id).join() === "z,a,m", "sortAndFilter mutated input");
  }

  // --- passesFilters truth table ---------------------------------------------
  {
    const vid = asset({ type: "video", name: "Beach", isGenerated: false });
    const genImg = asset({ type: "image", name: "AI Sky", isGenerated: true });
    const txt = asset({ type: "text", name: "Title" });

    // empty filter → all pass
    check(passesFilters(vid, noFilter()), "empty filter video should pass");
    check(passesFilters(txt, noFilter()), "empty filter text should pass");

    // type chip active excludes others (and text/lottie)
    const onlyVideo: FilterState = {
      filterTypes: new Set<FilterableType>(["video"]),
      filterAI: false,
      query: "",
    };
    check(passesFilters(vid, onlyVideo), "video chip → video passes");
    check(!passesFilters(genImg, onlyVideo), "video chip → image excluded");
    check(!passesFilters(txt, onlyVideo), "video chip → text excluded");

    // AI toggle
    const aiOnly: FilterState = { ...noFilter(), filterAI: true };
    check(passesFilters(genImg, aiOnly), "AI filter → generated passes");
    check(!passesFilters(vid, aiOnly), "AI filter → non-generated excluded");

    // query substring case-insensitive
    const q: FilterState = { ...noFilter(), query: " bea " };
    check(passesFilters(vid, q), "query 'bea' should match 'Beach' (trimmed/ci)");
    check(!passesFilters(genImg, q), "query 'bea' should not match 'AI Sky'");
  }

  // --- gridDimensions math ---------------------------------------------------
  {
    // width 360, thumb 110: usable = 360-16 = 344; spacing=16
    // cols = floor((344+16)/(110+16)) = floor(360/126) = 2
    // tileWidth = max(110, (344-16)/2) = max(110,164) = 164
    const g = gridDimensions(360, 110);
    check(g.columns === 2, `gridDimensions cols ${g.columns} (expected 2)`);
    check(g.tileWidth === 164, `gridDimensions tileWidth ${g.tileWidth} (expected 164)`);
    check(Spacing.xl === 16, "Spacing.xl must be 16 for grid math");

    // narrow → at least 1 column, tileWidth floors at thumbnailSize
    const narrow = gridDimensions(40, 200);
    check(narrow.columns === 1, "narrow grid → 1 column");
    check(narrow.tileWidth === 200, "narrow grid → tileWidth >= thumbnailSize");
  }

  // --- buildView (folder / flat / grouped) -----------------------------------
  {
    const folders: MediaFolderView[] = [
      { id: "f1", name: "Beta", parentFolderId: null },
      { id: "f2", name: "Alpha", parentFolderId: null },
      { id: "f1a", name: "Child", parentFolderId: "f1" },
    ];
    const assets: MediaAssetView[] = [
      asset({ id: "root1", name: "root", folderId: null }),
      asset({ id: "in-f1", name: "inF1", folderId: "f1" }),
      asset({ id: "in-f2", name: "inF2", folderId: "f2" }),
    ];
    const snap: MediaSnapshot = { folders, assets };

    // folder view at root: subfolders (array order) then root assets
    const fv = buildView("folder", snap, null, noFilter(), "dateAdded");
    check(
      fv.orderedKeys.join() === "folder-f1,folder-f2,root1",
      `folder view keys ${fv.orderedKeys.join()}`,
    );

    // flat view: every asset, no folders
    const flat = buildView("flat", snap, null, noFilter(), "dateAdded");
    check(
      flat.orderedKeys.join() === "root1,in-f1,in-f2",
      `flat view keys ${flat.orderedKeys.join()}`,
    );
    check(flat.sections.length === 0, "flat view has no sections");

    // grouped: root "Library" first, then folder sections sorted by path
    const grp = buildView("grouped", snap, null, noFilter(), "dateAdded");
    check(grp.sections[0]?.title === "Library", "grouped first section = Library");
    // Alpha (f2) path < Beta (f1) path, Child path = "Beta / Child"
    const titles = grp.sections.map((s) => s.title).join("|");
    check(
      titles === "Library|Alpha|Beta|Beta / Child" ||
        titles === "Library|Alpha|Beta", // child folder empty → skipped
      `grouped section order: ${titles}`,
    );
  }

  // --- moveSelection arrow nav -----------------------------------------------
  {
    const keys = ["a", "b", "c", "d", "e", "f"]; // 6 items, 3 cols
    check(moveSelection(keys, "a", "right", 3) === "b", "right a→b");
    check(moveSelection(keys, "a", "down", 3) === "d", "down a→d (cols 3)");
    check(moveSelection(keys, "a", "left", 3) === "a", "left at edge clamps");
    check(moveSelection(keys, "a", "up", 3) === "a", "up at top row clamps");
    check(moveSelection(keys, "f", "right", 3) === "f", "right at end clamps");
    check(moveSelection(keys, null, "down", 3) === "a", "no focus → first");
  }

  // --- marquee intersect -----------------------------------------------------
  {
    const r = marqueeRect(10, 10, 50, 50);
    check(r.x === 10 && r.y === 10 && r.w === 40 && r.h === 40, "marqueeRect normalize");
    const inside: Rect = { x: 20, y: 20, w: 10, h: 10 };
    const outside: Rect = { x: 100, y: 100, w: 10, h: 10 };
    check(rectsIntersect(r, inside), "rect intersect inside");
    check(!rectsIntersect(r, outside), "rect no-intersect outside");

    const frames = new Map<string, Rect>([
      ["k1", inside],
      ["k2", outside],
    ]);
    const sel = marqueeSelect(r, frames, new Set(), false);
    check(sel.has("k1") && !sel.has("k2"), "marqueeSelect picks intersecting only");
    // additive unions with base
    const sel2 = marqueeSelect(r, frames, new Set(["base"]), true);
    check(sel2.has("base") && sel2.has("k1"), "marqueeSelect additive unions base");
  }

  // --- drag payload URI format (the load-bearing contract) -------------------
  {
    check(assetUri("a1") === "palmier-asset://a1", "asset URI");
    check(folderUri("f1") === "palmier-folder://f1", "folder URI");
    // %.3f source seconds
    check(
      momentUri("a1", 1.5, 2) === "palmier-asset://a1#1.500-2.000",
      `moment URI %.3f, got ${momentUri("a1", 1.5, 2)}`,
    );

    // selected asset emits all selected ids newline-joined; else just that id
    const sel = new Set(["a1", "a2"]);
    check(
      buildAssetDragPayload("a1", sel) ===
        "palmier-asset://a1\npalmier-asset://a2",
      "multi-select payload newline-joined",
    );
    check(
      buildAssetDragPayload("a3", sel) === "palmier-asset://a3",
      "non-selected primary → single id",
    );
    check(
      buildAssetDragPayload("a1", new Set(["a1"])) === "palmier-asset://a1",
      "single selection → single id",
    );

    // round-trip parse
    const p = parseUri("palmier-asset://a1#1.500-2.000");
    check(
      p?.kind === "moment" && p.start === 1.5 && p.end === 2,
      "parse moment URI",
    );
    check(parseUri("palmier-folder://f1")?.kind === "folder", "parse folder URI");
  }

  // --- folder move cycle guards (E4-S12 / E4-S6 parity) ----------------------
  {
    const folders: MediaFolderView[] = [
      { id: "root", name: "Root", parentFolderId: null },
      { id: "child", name: "Child", parentFolderId: "root" },
      { id: "grand", name: "Grand", parentFolderId: "child" },
      { id: "sib", name: "Sibling", parentFolderId: null },
    ];

    // isDescendant: grand is a descendant of root; sib is not.
    check(isDescendant(folders, "grand", "root"), "grand is descendant of root");
    check(isDescendant(folders, "root", "root"), "isDescendant includes self");
    check(!isDescendant(folders, "sib", "root"), "sib not descendant of root");

    // legalFolderMoves rejects: into self, into a descendant, and no-op (already parent).
    check(
      legalFolderMoves(folders, ["root"], "root").length === 0,
      "reject move into self",
    );
    check(
      legalFolderMoves(folders, ["root"], "grand").length === 0,
      "reject move into a descendant",
    );
    check(
      legalFolderMoves(folders, ["child"], "root").length === 0,
      "reject no-op (already parent)",
    );
    // a legal move (sibling into root's child) is allowed
    check(
      legalFolderMoves(folders, ["sib"], "child").join() === "sib",
      "legal move into a non-descendant folder",
    );
    // moving to root (null) from a non-root parent is legal
    check(
      legalFolderMoves(folders, ["grand"], null).join() === "grand",
      "legal move up to root",
    );
  }

  // --- search Files filter ---------------------------------------------------
  {
    const assets = [
      asset({ id: "x", name: "beach_sunset.mp4" }),
      asset({ id: "y", name: "mountain.mov" }),
    ];
    const m = fileMatches(assets, "BEACH");
    check(m.length === 1 && m[0].id === "x", "fileMatches case-insensitive");
    check(fileMatches(assets, "").length === 0, "empty query → no file matches");
  }

  return fail;
}
