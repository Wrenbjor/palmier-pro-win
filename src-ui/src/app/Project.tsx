// Project window shell (E1-S4): the per-project editor window chrome (1600×1000 / min
// 960×600). The timeline/editor canvas itself is owned by another worker under
// `src-ui/src/editor/` — this shell only provides the window frame + the update badge +
// the mount point the editor surface plugs into. It does NOT implement editor content.
import UpdateBadge from "./UpdateBadge";

export default function Project({ projectId }: { projectId: string }) {
  return (
    <div className="flex h-screen flex-col bg-[#0a0a0a] text-white">
      <header className="flex items-center justify-between border-b border-white/10 px-4 py-2">
        <span className="text-sm text-white/60">Project</span>
        <UpdateBadge />
      </header>
      {/*
        Editor mount point. The timeline canvas worker (src-ui/src/editor/) renders here.
        Until that lands, the window is a valid, sized shell carrying the project id.
      */}
      <main className="flex flex-1 items-center justify-center text-white/40">
        <span data-project-id={projectId}>Editor loads here.</span>
      </main>
    </div>
  );
}
