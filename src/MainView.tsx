export default function MainView() {
  return (
    <div className="info-blurb">
      <h3 className="info-blurb-title">TSW Downloader</h3>
      <p>
        A community-built tool for downloading and updating
        The Secret World. Due to the original downloader being
        excessively slow by modern standards.
      </p>
      <p>
        Press <strong>DOWNLOAD</strong> below to install the game, or{" "}
        <strong>START GAME</strong> to launch the ClientPatcher
        once installation is complete.
      </p>
      <p className="info-blurb-note">
        This project is not affiliated with Funcom.
      </p>
    </div>
  );
}
