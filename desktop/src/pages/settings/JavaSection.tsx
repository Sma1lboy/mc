import { Panel, Heading, Spinner, ErrorState, Tag } from "../../components";
import { t } from "../../i18n";
import type { JavaInstall } from "../../ipc/types";

const sectionClass = "p-[20px]";

/** Java 检测列表块(数据由父页持有:下载设置的 Java 下拉也用它)。 */
export function JavaSection({
  javas,
}: {
  javas: { loading: boolean; data: JavaInstall[] | undefined; error: unknown; refetch: () => void };
}) {
  return (
            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionJava")}
              </Heading>
              {javas.loading ? (
                <div className="flex justify-center p-[20px]"><Spinner /></div>
              ) : (javas.data ?? []).length > 0 ? (
                <div className="flex flex-col gap-[8px]">
                  {(javas.data ?? []).map((j) => (
                    <Panel key={j.path} variant="raised" className="flex flex-col gap-[3px] px-[12px] py-[9px]">
                      <span className="flex items-center gap-[8px]">
                        <span className="font-display text-[14px] text-strong">Java {j.version}</span>
                        <Tag>{j.is_64bit ? t("settings.bit64") : t("settings.bit32")}</Tag>
                        <span className="text-[12px] text-accent">{j.source}</span>
                      </span>
                      <span className="text-[11px] text-faint break-all">{j.path}</span>
                    </Panel>
                  ))}
                </div>
              ) : javas.error ? (
                <ErrorState compact message={t("settings.javaDetectFailed")} onRetry={() => javas.refetch()} />
              ) : (
                <div className="text-muted">{t("settings.noJava")}</div>
              )}
            </Panel>
  );
}
