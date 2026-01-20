import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";

const GITHUB_URL = "https://github.com/my-org-10/ClaudeCodeMonitor";
const TWITTER_URL = "https://x.com/Siddartha_10";

export function AboutView() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    getVersion()
      .then((value) => {
        if (active) {
          setVersion(value);
        }
      })
      .catch(() => {
        if (active) {
          setVersion(null);
        }
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <div className="about">
      <div className="about-card">
        <div className="about-header">
          <img
            className="about-icon"
            src="/app-icon.png"
            alt="Claude Code Monitor icon"
          />
          <div className="about-title">Claude Code Monitor</div>
        </div>
        <div className="about-version">
          {version ? `Version ${version}` : "Version —"}
        </div>
        <div className="about-tagline">
          Monitor the situation of your Claude agents
        </div>
        <div className="about-divider" />
        <div className="about-links">
          <button
            type="button"
            className="about-link"
            onClick={() => openUrl(GITHUB_URL)}
          >
            GitHub
          </button>
          <span className="about-link-sep">|</span>
          <button
            type="button"
            className="about-link"
            onClick={() => openUrl(TWITTER_URL)}
          >
            Twitter
          </button>
        </div>
        <div className="about-footer">Made with care by Driod $ Siddartha</div>
      </div>
    </div>
  );
}
