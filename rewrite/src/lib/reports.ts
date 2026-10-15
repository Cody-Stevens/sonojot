import { z } from "zod";

export const reportKindSchema = z.enum(["crash", "bug"]);
const appSchema = z
  .object({
    version: z.string().min(1).max(80),
    platform: z.enum(["windows", "macos", "linux"]),
    arch: z.string().min(1).max(40),
    os: z.string().max(200),
  })
  .strict();
const crashSchema = z
  .object({
    summary: z.string().max(2000),
    location: z.string().max(300),
    backtrace: z.string().max(16 * 1024),
  })
  .strict();
export const pendingCrashSchema = z
  .object({
    id: z.uuidv4(),
    reason: z.enum(["panic", "unexpectedExit"]),
    app: appSchema,
    crash: crashSchema,
  })
  .strict();
export const reportSchema = z
  .object({
    schema: z.literal(1),
    kind: reportKindSchema,
    requestId: z.uuidv4(),
    app: appSchema,
    // Empty notes are valid while drafting; bug submission requires nonblank notes.
    notes: z.string().refine((s) => Array.from(s).length <= 4000),
    crash: crashSchema.nullable(),
    diagnostics: z.record(z.string(), z.json()).nullable(),
  })
  .strict();
export const reportReceiptSchema = z
  .object({
    receiptId: z.string().min(1).max(160),
    replayed: z.boolean(),
  })
  .strict();
export type Report = z.infer<typeof reportSchema>;
export type ReportKind = z.infer<typeof reportKindSchema>;
export type PendingCrash = z.infer<typeof pendingCrashSchema>;

export function reportCanSend(
  report: Report | null,
  consent: boolean,
): boolean {
  return (
    report !== null &&
    (report.kind === "crash" ? consent : report.notes.trim().length > 0) &&
    Array.from(report.notes).length <= 4000 &&
    new TextEncoder().encode(JSON.stringify(report)).length <= 64 * 1024
  );
}

export function crashHeading(reason: PendingCrash["reason"]): string {
  return reason === "panic" ? "SonoJot crashed" : "SonoJot closed unexpectedly";
}

export function shouldShowCrash(
  pending: PendingCrash | null,
  setupChecked: boolean,
  onboarding: boolean,
  termsRequired: boolean,
): boolean {
  return pending !== null && setupChecked && !onboarding && !termsRequired;
}
