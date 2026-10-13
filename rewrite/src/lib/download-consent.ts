import { z } from "zod";
import catalog from "../../legal/download-consent.json";

export const downloadConsentSchema = z.object({
  id: z.string(),
  version: z.string(),
  models: z.array(z.string()),
  platform: z.string(),
  title: z.string(),
  summary: z.string(),
  checkbox: z.string(),
  components: z.array(z.object({ name: z.string(), version: z.string() })),
  links: z.array(z.object({ label: z.string(), url: z.string().url() })),
});
export type DownloadConsent = z.infer<typeof downloadConsentSchema>;

export const downloadConsentGroups = z
  .array(downloadConsentSchema)
  .parse(catalog.groups);

export function pendingDownloadConsents(
  ids: string[],
  platform: string,
  accepted: Record<string, string>,
): DownloadConsent[] {
  return downloadConsentGroups.filter(
    (group) =>
      (group.platform === "all" || group.platform === platform) &&
      group.models.some((id) => ids.includes(id)) &&
      accepted[group.id] !== group.version,
  );
}
