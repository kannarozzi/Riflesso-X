import { useState } from "react";
import { TriangleAlert } from "lucide-react";
import { restartReceiver, showMirror, type Device, type Snapshot } from "../backend";
import { useT } from "../i18n";

export function DevicesPanel({ snapshot }: { snapshot: Snapshot }) {
  const t = useT();
  const { status, devices, requested } = snapshot;
  const main = devices.find((d) => d.mirroring) ?? devices[0] ?? null;
  const others = devices.filter((d) => d !== main);

  return (
    <div className="flex flex-1 flex-col items-center">
      <Stage device={main} waiting={status.state === "running" && !main} />

      <div className="mt-1 w-full">
        {status.state === "error" ? (
          <ErrorCard message={status.message} />
        ) : main ? (
          <DeviceInfo device={main} />
        ) : (
          <EmptyState name={snapshot.settings.name} />
        )}
      </div>

      {others.length > 0 && (
        <ul className="mt-5 w-full divide-y divide-edge border-y border-edge">
          {others.map((d) => (
            <li key={d.id} className="flex justify-between py-2 text-[13px]">
              <span className="truncate">{d.name}</span>
              <span className="shrink-0 text-dim">{t(d.mirroring ? "devices.mirroring" : "devices.connected")}</span>
            </li>
          ))}
        </ul>
      )}

      <p className="mt-auto pt-5 font-mono text-[11px] text-faint">
        {t("devices.requested", {
          size: `${requested.width}×${requested.height}`,
          codec: requested.hevc ? "H.265" : "H.264",
        })}
      </p>
    </div>
  );
}

// ---- The mirrored screen standing on a horizon line, with its reflection ----

const STAGE_W = 220;
const STAGE_H = 188;

const kindAspect = { iphone: 0.46, ipad: 0.75, mac: 1.6, other: 0.46 };

/** Screen size on the stage: the real aspect ratio of the stream when there is one. */
function screenSize(device: Device | null) {
  const aspect = device?.video ? device.video.width / device.video.height : kindAspect[device?.kind ?? "iphone"];
  return aspect < STAGE_W / STAGE_H
    ? { width: Math.round(STAGE_H * aspect), height: STAGE_H }
    : { width: STAGE_W, height: Math.round(STAGE_W / aspect) };
}

function Stage({ device, waiting }: { device: Device | null; waiting: boolean }) {
  const size = screenSize(device);
  const lit = !!device?.mirroring;
  const island = (device?.kind ?? "iphone") === "iphone";
  return (
    <div className="flex flex-col items-center" aria-hidden>
      <div className="flex items-end" style={{ height: STAGE_H }}>
        <Screen size={size} lit={lit} island={island} />
      </div>
      <div className="relative h-px w-[220px] overflow-hidden bg-linear-to-r from-transparent via-ink/35 to-transparent">
        {(waiting || lit) && (
          <span
            key={String(lit)}
            className="absolute inset-y-0 left-0 w-2/5 bg-linear-to-r from-transparent via-white to-transparent"
            style={{ animation: lit ? "sheen 1.2s ease-out 0.4s both" : "sheen 4s ease-in-out infinite" }}
          />
        )}
      </div>
      <div className="reflection flex h-[70px] items-end overflow-hidden opacity-25">
        <Screen size={size} lit={lit} island={island} />
      </div>
    </div>
  );
}

function Screen({ size, lit, island }: { size: { width: number; height: number }; lit: boolean; island: boolean }) {
  const portrait = size.height >= size.width;
  return (
    <div
      className="relative shrink-0 overflow-hidden border-[2.5px] border-ink bg-glass transition-[width,height] duration-500 ease-out"
      style={{ ...size, borderRadius: Math.min(size.width, size.height) * 0.24 }}
    >
      {lit && <div className="bg-iris-screen absolute inset-0" style={{ animation: "power-on 0.7s ease-out" }} />}
      {island && (
        <span
          className={`absolute rounded-full bg-black ${
            portrait ? "top-2 left-1/2 h-[7px] w-[26px] -translate-x-1/2" : "top-1/2 left-2 h-[26px] w-[7px] -translate-y-1/2"
          }`}
        />
      )}
    </div>
  );
}

// ---- Text under the stage ----

function DeviceInfo({ device }: { device: Device }) {
  const t = useT();
  return (
    <div className="flex flex-col items-center text-center">
      <h2 className="text-[22px] font-semibold tracking-tight">{device.name}</h2>
      <p className="mt-1 font-mono text-xs text-dim">
        {device.mirroring && device.video
          ? `${device.video.codec} · ${device.video.width}×${device.video.height}`
          : t("devices.notMirroring")}
      </p>
      {device.mirroring && (
        <button
          onClick={() => showMirror()}
          className="mt-4 rounded-full bg-ink px-5 py-2 text-[13px] font-semibold text-on-ink shadow-[0_6px_16px_-6px_rgb(14_18_24/0.6)] transition hover:opacity-90"
        >
          {t("devices.show")}
        </button>
      )}
    </div>
  );
}

function EmptyState({ name }: { name: string }) {
  const t = useT();
  const steps = [t("empty.step1"), t("empty.step2"), t("empty.step3", { name })];
  return (
    <div>
      <h2 className="text-center text-[22px] font-semibold tracking-tight">{t("empty.title")}</h2>
      <p className="mt-1 text-center text-[13px] text-dim">{t("empty.visibleAs", { name })}</p>
      <ol className="mt-5 space-y-2.5">
        {steps.map((step, i) => (
          <li key={i} className="flex gap-2.5 text-[13px] text-ink-soft">
            <span className="grid size-5 shrink-0 place-items-center rounded-full bg-glass text-[11px] font-medium text-ink shadow-[0_0_0_1px_var(--color-edge)]">
              {i + 1}
            </span>
            <span className="pt-px">{step}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}

function ErrorCard({ message }: { message: string }) {
  const t = useT();
  const [retrying, setRetrying] = useState(false);
  const portBusy = message.includes("7000");
  return (
    <div className="rounded-2xl bg-danger/10 p-4 ring-1 ring-danger/25">
      <div className="flex gap-3">
        <TriangleAlert className="mt-0.5 size-5 shrink-0 text-danger" />
        <div className="min-w-0 space-y-2 text-[13px]">
          <p className="font-semibold text-danger">{t("error.title")}</p>
          <p className="text-ink-soft">{portBusy ? t("error.portBusy") : message}</p>
          <button
            disabled={retrying}
            onClick={async () => {
              setRetrying(true);
              await restartReceiver().catch(() => {});
              setRetrying(false);
            }}
            className="rounded-full bg-ink px-4 py-1.5 font-semibold text-on-ink transition hover:opacity-90 disabled:opacity-50"
          >
            {t(retrying ? "error.retrying" : "error.retry")}
          </button>
        </div>
      </div>
    </div>
  );
}
