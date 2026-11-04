import { useEffect, useMemo, useState } from "react";
import { engine, isPreview, type UpdateView } from "@/engine/bridge";

/** The shell checks on its own clock; this only reads the state it keeps. */
const POLL_MS = 4000;

const megabytes = (bytes: number) => `${(bytes / (1024 * 1024)).toFixed(1)} MB`;

/**
 * The one row that appears under the header when the release endpoint offers a
 * newer build.
 *
 * Nothing installs without the click: the shell holds the version, this shows it
 * with the first line of the notes, and hides it again for a version the person
 * has already said not to now. A failed check shows nothing at all, because an
 * update nobody asked for must not turn into a warning on every launch.
 */
export function UpdateBanner() {
  const [update, setUpdate] = useState<UpdateView | null>(null);
  /** The version somebody chose to skip until the next launch. */
  const [later, setLater] = useState<string | null>(null);

  useEffect(() => {
    if (isPreview) return;
    let live = true;
    const read = async () => {
      const next = await engine.updateState().catch(() => null);
      if (live && next) setUpdate(next);
    };
    void read();
    const id = window.setInterval(read, POLL_MS);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, []);

  const working = useMemo(
    () =>
      update
        ? update.status === "downloading" ||
          update.status === "installing" ||
          update.status === "restarting"
        : false,
    [update],
  );

  if (!update || !update.version) return null;
  if (update.status !== "available" && !working) return null;
  if (later === update.version) return null;

  const progress = update.total
    ? `${megabytes(update.downloaded)} of ${megabytes(update.total)}`
    : megabytes(update.downloaded);

  return (
    <div className="sp__update" role="status">
      <span className="sp__update-mark" aria-hidden="true">
        ↑
      </span>
      <div className="sp__update-text">
        <p className="sp__update-title">Spud {update.version} is available</p>
        <p className="sp__update-note">
          {working
            ? `Downloading ${progress}…`
            : (update.notes?.split("\n")[0] ??
              "A newer build is ready to replace this one.")}
        </p>
      </div>
      {working ? (
        <span className="sp__update-state">
          {update.status === "downloading" ? progress : "installing…"}
        </span>
      ) : (
        <>
          <button
            type="button"
            className="sp__update-go"
            onClick={() => {
              void engine.updateInstall();
            }}
          >
            Update now
          </button>
          <button
            type="button"
            className="sp__update-later"
            onClick={() => setLater(update.version)}
          >
            Later
          </button>
        </>
      )}
    </div>
  );
}
