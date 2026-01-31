import FolderPlus from "lucide-react/dist/esm/icons/folder-plus";

type SidebarHeaderProps = {
  onSelectHome: () => void;
  onAddWorkspace: () => void;
};

export function SidebarHeader({
  onSelectHome,
  onAddWorkspace,
}: SidebarHeaderProps) {
  return (
    <div className="sidebar-header">
      <div className="sidebar-header-title">
        <div className="sidebar-title-group">
          <button
            className="sidebar-title-add"
            onClick={onAddWorkspace}
            data-tauri-drag-region="false"
            aria-label="Add workspace"
            type="button"
          >
            <FolderPlus aria-hidden />
          </button>
          <button
            className="subtitle subtitle-button sidebar-title-button"
            onClick={onSelectHome}
            data-tauri-drag-region="false"
            aria-label="Open home"
          >
            Projects
          </button>
        </div>
      </div>
    </div>
  );
}
