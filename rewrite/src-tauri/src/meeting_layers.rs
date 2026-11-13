//! Local meeting documents. Recognition is immutable; publication is a projection.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: u32 = 1;
pub type Clock = BTreeMap<String, u64>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub lamport: u64,
    pub device: String,
    pub batch: String,
    pub index: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub sid: String,
    pub text: String,
    pub label: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Raw {
    pub schema: u32,
    pub segments: Vec<Line>,
    /// Exact saved recognition, including timed segments, guarded text and diagnostics.
    pub recognition: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clean {
    pub schema: u32,
    pub base: String,
    pub segments: Vec<Line>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum Op {
    Set {
        sid: String,
        text: String,
    },
    Insert {
        after: Option<String>,
        id: String,
        text: String,
    },
    Delete {
        sid: String,
    },
    Label {
        sid: String,
        label: Option<String>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub schema: u32,
    pub base: String,
    /// "original", or the immutable clean-layer id.
    pub view: String,
    pub lamport: u64,
    pub device: String,
    pub seen: Clock,
    pub ops: Vec<Op>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub stamp: Stamp,
    pub seen: Clock,
    pub value: Option<String>,
}
fn observes(a: &Event, b: &Event) -> bool {
    (a.stamp.device == b.stamp.device && a.stamp > b.stamp)
        || a.seen.get(&b.stamp.device).copied().unwrap_or(0) >= b.stamp.lamport
}
fn register(events: &mut Vec<Event>, incoming: Event) {
    if events
        .iter()
        .any(|e| e.stamp == incoming.stamp || observes(e, &incoming))
    {
        return;
    }
    events.retain(|e| !observes(&incoming, e));
    events.push(incoming);
    events.sort_by(|a, b| a.stamp.cmp(&b.stamp));
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub line: Line,
    pub after: Option<String>,
    pub inserted: Stamp,
    pub text: Vec<Event>,
    pub labels: Vec<Event>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub base: String,
    pub view: String,
    /// Exact batch ids, not a scalar watermark: late offline batches still apply.
    pub folded: BTreeSet<String>,
    pub clock: Clock,
    pub rows: BTreeMap<String, Row>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Head {
    pub schema: u32,
    pub id: String,
    pub raw: String,
    pub clean: Option<String>,
    pub notes: Vec<String>,
    pub snapshots: BTreeMap<String, String>,
    pub fields: BTreeMap<String, Value>,
    pub audio: Option<crate::meeting_sync_audio::Manifest>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layers {
    pub schema: u32,
    pub head: Head,
    pub raw: BTreeMap<String, Raw>,
    pub clean: BTreeMap<String, Clean>,
    pub batches: BTreeMap<String, Batch>,
    pub snapshots: BTreeMap<String, Snapshot>,
    pub notes: BTreeMap<String, Value>,
    pub clock: Clock,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub sid: String,
    pub view: String,
    pub version: String,
    pub text: String,
}

pub fn lines(text: &str, prefix: &str) -> Vec<Line> {
    text.split_inclusive('\n')
        .enumerate()
        .map(|(i, text)| Line {
            sid: format!("{prefix}-{i}"),
            text: text.into(),
            label: None,
        })
        .collect()
}
impl Head {
    pub fn validate_fields(&self) -> Result<(), String> {
        for (key, value) in &self.fields {
            if !matches!(
                key.as_str(),
                "title"
                    | "createdAt"
                    | "durationSeconds"
                    | "status"
                    | "source"
                    | "error"
                    | "recognitionCompleteness"
                    | "recognitionWarnings"
                    | "recordingWarnings"
                    | "audioOutages"
                    | "missingSeconds"
                    | "transcriptionStoppedAtSeconds"
                    | "transcriptionModel"
                    | "speakerCount"
                    | "speakerLabels"
                    | "record.speakerNames"
                    | "record.titleUserEdited"
                    | "record.automaticTitlePending"
                    | "record.recognitionFile"
                    | "record.importSpeaker"
                    | "record.outagesRecorded"
                    | "record.rawTranscriptRef"
            ) {
                return Err("Unsupported synced meeting field.".into());
            }
            if key == "record.recognitionFile" && !value.is_null() {
                let file = value
                    .as_str()
                    .and_then(|v| v.strip_suffix(".json"))
                    .ok_or("Invalid recognition filename.")?;
                crate::store::validate_id(file)?;
            }
            if key == "status"
                && !matches!(
                    value.as_str(),
                    Some(
                        "ready"
                            | "transcribed"
                            | "processed"
                            | "failed"
                            | "interrupted"
                            | "cancelled"
                    )
                )
            {
                return Err("A synced meeting has an invalid status.".into());
            }
        }
        Ok(())
    }
}
impl Layers {
    pub fn empty(id: &str) -> Self {
        Self {
            schema: SCHEMA,
            head: Head {
                schema: SCHEMA,
                id: id.into(),
                raw: String::new(),
                clean: None,
                notes: vec![],
                snapshots: BTreeMap::new(),
                fields: BTreeMap::new(),
                audio: None,
            },
            raw: BTreeMap::new(),
            clean: BTreeMap::new(),
            batches: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            notes: BTreeMap::new(),
            clock: BTreeMap::new(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA
            || self.head.schema != SCHEMA
            || !self.raw.contains_key(&self.head.raw)
            || self
                .head
                .clean
                .as_ref()
                .is_some_and(|id| !self.clean.contains_key(id))
            || self
                .head
                .notes
                .iter()
                .any(|id| !self.notes.contains_key(id))
        {
            return Err("Update SonoJot before opening these meeting layers.".into());
        }
        crate::store::validate_id(&self.head.id)?;
        self.head.validate_fields()?;
        for batch in self.batches.values() {
            if batch.schema != SCHEMA
                || batch.device.is_empty()
                || batch.lamport == 0
                || batch.seen.values().any(|seen| *seen >= batch.lamport)
                || !self.raw.contains_key(&batch.base)
            {
                return Err("Invalid meeting edit batch.".into());
            }
        }
        for id in self.head.snapshots.values() {
            if !self.snapshots.contains_key(id) {
                return Err("A meeting snapshot is missing.".into());
            }
        }
        for (id, raw) in &self.raw {
            crate::store::validate_id(id)?;
            if raw.schema != SCHEMA {
                return Err("Unsupported raw recognition schema.".into());
            }
        }
        for (id, clean) in &self.clean {
            crate::store::validate_id(id)?;
            if clean.schema != SCHEMA || !self.raw.contains_key(&clean.base) {
                return Err("Unsupported clean layer.".into());
            }
        }
        if let Some(audio) = &self.head.audio {
            audio.validate()?;
        }
        Ok(())
    }
    pub fn state(&self, view: &str) -> Result<Snapshot, String> {
        let mut state = if let Some(id) = self.head.snapshots.get(view) {
            let s = self.snapshots.get(id).ok_or("Missing meeting snapshot.")?;
            if s.schema != SCHEMA || s.base != self.head.raw || s.view != view {
                return Err("Meeting snapshot has a different base.".into());
            }
            s.clone()
        } else {
            let source = if view == "original" {
                &self
                    .raw
                    .get(&self.head.raw)
                    .ok_or("Missing raw recognition.")?
                    .segments
            } else {
                &self.clean.get(view).ok_or("Missing clean layer.")?.segments
            };
            let mut s = Snapshot {
                schema: SCHEMA,
                base: self.head.raw.clone(),
                view: view.into(),
                ..Default::default()
            };
            let mut after = None;
            for line in source {
                s.rows.insert(
                    line.sid.clone(),
                    Row {
                        line: line.clone(),
                        after: after.clone(),
                        inserted: Stamp::default(),
                        text: vec![],
                        labels: vec![],
                    },
                );
                after = Some(line.sid.clone());
            }
            s
        };
        // Concurrent compactions may each fold edits absent from the other's
        // snapshot. Join their causal registers before skipping any folded id.
        for other in self
            .snapshots
            .values()
            .filter(|s| s.base == self.head.raw && s.view == view)
        {
            if other.schema != SCHEMA {
                return Err("Unsupported snapshot schema.".into());
            }
            for (id, row) in &other.rows {
                if let Some(existing) = state.rows.get_mut(id) {
                    if existing.inserted != row.inserted
                        || existing.after != row.after
                        || existing.line != row.line
                    {
                        return Err("Snapshot changed an immutable segment.".into());
                    }
                    for event in &row.text {
                        register(&mut existing.text, event.clone());
                    }
                    for event in &row.labels {
                        register(&mut existing.labels, event.clone());
                    }
                } else {
                    state.rows.insert(id.clone(), row.clone());
                }
            }
            state.folded.extend(other.folded.iter().cloned());
            for (device, mark) in &other.clock {
                state
                    .clock
                    .entry(device.clone())
                    .and_modify(|n| *n = (*n).max(*mark))
                    .or_insert(*mark);
            }
        }
        let mut batches: Vec<_> = self
            .batches
            .iter()
            .filter(|(id, b)| {
                b.base == self.head.raw && b.view == view && !state.folded.contains(*id)
            })
            .collect();
        batches.sort_by_key(|(id, b)| (b.lamport, &b.device, *id));
        // Insert first, so late offline edits can target an already compacted insertion.
        for (id, b) in &batches {
            for (index, op) in b.ops.iter().enumerate() {
                if let Op::Insert {
                    after,
                    id: sid,
                    text,
                } = op
                {
                    let row = Row {
                        line: Line {
                            sid: sid.clone(),
                            text: text.clone(),
                            label: None,
                        },
                        after: after.clone(),
                        inserted: Stamp {
                            lamport: b.lamport,
                            device: b.device.clone(),
                            batch: (*id).clone(),
                            index,
                        },
                        text: vec![],
                        labels: vec![],
                    };
                    if let Some(existing) = state.rows.get(sid) {
                        if existing.inserted != row.inserted {
                            return Err("Duplicate meeting segment id.".into());
                        }
                    } else {
                        state.rows.insert(sid.clone(), row);
                    }
                }
            }
        }
        for (id, b) in batches {
            for (index, op) in b.ops.iter().enumerate() {
                let (sid, value, label) = match op {
                    Op::Set { sid, text } => (sid, Some(text.clone()), false),
                    Op::Delete { sid } => (sid, None, false),
                    Op::Label { sid, label } => (sid, label.clone(), true),
                    Op::Insert { .. } => continue,
                };
                let row = state
                    .rows
                    .get_mut(sid)
                    .ok_or("Meeting edit references a missing segment.")?;
                let event = Event {
                    stamp: Stamp {
                        lamport: b.lamport,
                        device: b.device.clone(),
                        batch: id.clone(),
                        index,
                    },
                    seen: b.seen.clone(),
                    value,
                };
                register(
                    if label {
                        &mut row.labels
                    } else {
                        &mut row.text
                    },
                    event,
                );
            }
            state
                .clock
                .entry(b.device.clone())
                .and_modify(|n| *n = (*n).max(b.lamport))
                .or_insert(b.lamport);
            state.folded.insert(id.clone());
        }
        Ok(state)
    }
    pub fn rendered(&self, view: &str) -> Result<(Vec<Line>, Vec<Conflict>), String> {
        let state = self.state(view)?;
        let mut children: BTreeMap<Option<String>, Vec<&Row>> = BTreeMap::new();
        for row in state.rows.values() {
            children.entry(row.after.clone()).or_default().push(row);
        }
        for children in children.values_mut() {
            children.sort_by(|a, b| {
                b.inserted
                    .cmp(&a.inserted)
                    .then(a.line.sid.cmp(&b.line.sid))
            });
        }
        let mut stack: Vec<&Row> = children
            .get(&None)
            .into_iter()
            .flatten()
            .rev()
            .copied()
            .collect();
        let mut visited = BTreeSet::new();
        let mut lines = Vec::new();
        let mut conflicts = Vec::new();
        while let Some(row) = stack.pop() {
            if !visited.insert(&row.line.sid) {
                return Err("Meeting segment order contains a cycle.".into());
            }
            let mut line = row.line.clone();
            // Concurrent delete/set retains text; a causal delete still removes it.
            let texts: Vec<_> = row.text.iter().filter(|e| e.value.is_some()).collect();
            let visible = row.text.is_empty() || !texts.is_empty();
            if let Some(winner) = texts.last() {
                line.text = winner.value.clone().unwrap();
                for other in texts.iter().filter(|other| other.value != winner.value) {
                    conflicts.push(Conflict {
                        sid: line.sid.clone(),
                        view: view.into(),
                        version: format!("{}:{}", other.stamp.batch, other.stamp.index),
                        text: other.value.clone().unwrap(),
                    });
                }
            }
            if let Some(event) = row.labels.last() {
                line.label = event.value.clone();
            }
            if visible {
                lines.push(line);
            }
            if let Some(next) = children.get(&Some(row.line.sid.clone())) {
                stack.extend(next.iter().rev().copied());
            }
        }
        if visited.len() != state.rows.len() {
            return Err("Meeting segment order is incomplete.".into());
        }
        Ok((lines, conflicts))
    }
    pub fn text(&self, view: &str) -> Result<String, String> {
        Ok(self
            .rendered(view)?
            .0
            .into_iter()
            .map(|line| match line.label {
                Some(label) => {
                    // Labels are presentation only. The text register and raw stay unchanged.
                    if let Some((prefix, rest)) = line.text.split_once(": ") {
                        let clock = prefix
                            .rsplit_once(' ')
                            .filter(|(s, _)| s.contains(':'))
                            .map(|(s, _)| format!("{s} "))
                            .unwrap_or_default();
                        format!("{clock}{label}: {rest}")
                    } else {
                        format!("{label}: {}", line.text)
                    }
                }
                None => line.text,
            })
            .collect())
    }
    pub fn add_ops(
        &mut self,
        device: &str,
        view: &str,
        ops: Vec<Op>,
    ) -> Result<Option<String>, String> {
        if ops.is_empty() {
            return Ok(None);
        }
        let lamport = self
            .clock
            .values()
            .copied()
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Meeting clock exhausted.")?;
        let id = uuid::Uuid::new_v4().to_string();
        self.batches.insert(
            id.clone(),
            Batch {
                schema: SCHEMA,
                base: self.head.raw.clone(),
                view: view.into(),
                lamport,
                device: device.into(),
                seen: self.clock.clone(),
                ops,
            },
        );
        self.clock.insert(device.into(), lamport);
        Ok(Some(id))
    }
    pub fn edit(&mut self, device: &str, view: &str, text: &str) -> Result<(), String> {
        let before = self.rendered(view)?.0;
        let old: Vec<&str> = before.iter().map(|l| l.text.as_str()).collect();
        let new: Vec<&str> = text.split_inclusive('\n').collect();
        let mut ops = Vec::new();
        let mut after = None;
        for change in similar::capture_diff_slices(similar::Algorithm::Myers, &old, &new) {
            let (tag, old_range, new_range) = change.as_tag_tuple();
            if tag == similar::DiffTag::Equal {
                after = old_range.last().map(|i| before[i].sid.clone()).or(after);
                continue;
            }
            let paired = old_range.len().min(new_range.len());
            for offset in 0..paired {
                let sid = before[old_range.start + offset].sid.clone();
                ops.push(Op::Set {
                    sid: sid.clone(),
                    text: new[new_range.start + offset].into(),
                });
                after = Some(sid);
            }
            for i in old_range.start + paired..old_range.end {
                ops.push(Op::Delete {
                    sid: before[i].sid.clone(),
                });
            }
            for text in &new[new_range.start + paired..new_range.end] {
                let id = uuid::Uuid::new_v4().to_string();
                ops.push(Op::Insert {
                    after: after.clone(),
                    id: id.clone(),
                    text: (*text).into(),
                });
                after = Some(id);
            }
        }
        self.add_ops(device, view, ops)?;
        Ok(())
    }
    pub fn compact(&mut self, view: &str) -> Result<bool, String> {
        let current = self
            .head
            .snapshots
            .get(view)
            .and_then(|id| self.snapshots.get(id));
        let pending: Vec<_> = self
            .batches
            .iter()
            .filter(|(id, b)| {
                b.base == self.head.raw
                    && b.view == view
                    && !current.is_some_and(|s| s.folded.contains(*id))
            })
            .collect();
        if pending.len() < 200
            && serde_json::to_vec(&pending)
                .map_err(|e| e.to_string())?
                .len()
                < 65536
        {
            return Ok(false);
        }
        let state = self.state(view)?;
        let id = uuid::Uuid::new_v4().to_string();
        self.snapshots.insert(id.clone(), state);
        self.head.snapshots.insert(view.into(), id);
        Ok(true)
    }
    pub fn merge_immutable(&mut self, remote: &Self) -> Result<(), String> {
        macro_rules! union {
            ($field:ident) => {
                for (id, value) in &remote.$field {
                    if self.$field.get(id).is_some_and(|old| old != value) {
                        return Err("Immutable meeting data changed.".into());
                    }
                    self.$field.insert(id.clone(), value.clone());
                }
            };
        }
        union!(raw);
        union!(clean);
        union!(batches);
        union!(snapshots);
        for (device, mark) in &remote.clock {
            self.clock
                .entry(device.clone())
                .and_modify(|n| *n = (*n).max(*mark))
                .or_insert(*mark);
        }
        for batch in remote.batches.values() {
            self.clock
                .entry(batch.device.clone())
                .and_modify(|n| *n = (*n).max(batch.lamport))
                .or_insert(batch.lamport);
        }
        for snapshot in remote.snapshots.values() {
            for (device, mark) in &snapshot.clock {
                self.clock
                    .entry(device.clone())
                    .and_modify(|n| *n = (*n).max(*mark))
                    .or_insert(*mark);
            }
        }
        Ok(())
    }
}

/// Three-way recursive field merge. Remote wins a collision; retain the local value.
pub fn merge_fields(
    base: &Value,
    local: &Value,
    remote: &Value,
    path: &str,
    conflicts: &mut BTreeMap<String, Value>,
) -> Value {
    if local == remote || local == base {
        return remote.clone();
    }
    if remote == base {
        return local.clone();
    }
    if let (Some(l), Some(r)) = (local.as_object(), remote.as_object()) {
        let keys: BTreeSet<_> = l
            .keys()
            .chain(r.keys())
            .chain(base.as_object().into_iter().flat_map(|b| b.keys()))
            .collect();
        let mut result = serde_json::Map::new();
        for key in keys {
            let value = merge_fields(
                &base[key],
                &local[key],
                &remote[key],
                &format!("{path}/{}", key.replace('~', "~0").replace('/', "~1")),
                conflicts,
            );
            if !value.is_null() || l.contains_key(key) || r.contains_key(key) {
                result.insert(key.clone(), value);
            }
        }
        return Value::Object(result);
    }
    // A field deletion concurrent with an edit keeps the edit.
    if local.is_null() {
        return remote.clone();
    }
    if remote.is_null() {
        return local.clone();
    }
    conflicts.insert(path.into(), local.clone());
    remote.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn document() -> Layers {
        let mut d = Layers::empty("synthetic");
        d.head.raw = "raw-one".into();
        d.raw.insert(
            "raw-one".into(),
            Raw {
                schema: SCHEMA,
                segments: lines("One.\nTwo.\nThree.", "r"),
                recognition: json!({"raw":"One.\nTwo.\nThree.","warnings":["synthetic"]}),
            },
        );
        d
    }
    #[test]
    fn edits_preserve_raw_and_whitespace() {
        let mut d = document();
        let raw = d.raw.clone();
        for text in [
            "One.\nChanged.\nThree.",
            "Prefix\nOne.\n\nChanged.\nThree.\nTail\n",
            "",
            "new\r\nlines\n",
        ] {
            d.edit("a", "original", text).unwrap();
            assert_eq!(d.text("original").unwrap(), text);
        }
        assert_eq!(d.raw, raw);
    }
    #[test]
    fn order_conflicts_and_causal_resolution_converge() {
        let mut a = document();
        let mut b = a.clone();
        a.edit("a", "original", "Left.\nTwo.\nThree.").unwrap();
        b.edit("b", "original", "Right.\nTwo.\nThree.").unwrap();
        let aa = a.clone();
        a.merge_immutable(&b).unwrap();
        b.merge_immutable(&aa).unwrap();
        assert_eq!(a.text("original"), b.text("original"));
        assert_eq!(a.rendered("original").unwrap().1.len(), 1);
        a.edit("a", "original", "Resolved.\nTwo.\nThree.").unwrap();
        b.merge_immutable(&a).unwrap();
        assert!(b.rendered("original").unwrap().1.is_empty());
        assert_eq!(b.text("original").unwrap(), "Resolved.\nTwo.\nThree.");
    }
    #[test]
    fn concurrent_delete_keeps_edit_but_causal_delete_wins() {
        let mut a = document();
        let mut b = a.clone();
        a.add_ops("a", "original", vec![Op::Delete { sid: "r-0".into() }])
            .unwrap();
        b.edit("b", "original", "Kept.\nTwo.\nThree.").unwrap();
        a.merge_immutable(&b).unwrap();
        assert!(a.text("original").unwrap().starts_with("Kept."));
        a.add_ops("a", "original", vec![Op::Delete { sid: "r-0".into() }])
            .unwrap();
        assert_eq!(a.text("original").unwrap(), "Two.\nThree.");
    }
    #[test]
    fn compaction_keeps_late_offline_edits_and_conflicts() {
        let mut a = document();
        let mut offline = a.clone();
        offline
            .edit("b", "original", "Offline.\nTwo.\nThree.")
            .unwrap();
        for n in 0..201 {
            a.edit("a", "original", &format!("Edit {n}.\nTwo.\nThree."))
                .unwrap();
        }
        assert!(a.compact("original").unwrap());
        let expected = a.text("original").unwrap();
        a.batches.clear();
        assert_eq!(a.text("original").unwrap(), expected);
        a.merge_immutable(&offline).unwrap();
        assert_eq!(a.text("original").unwrap(), expected);
        assert_eq!(a.rendered("original").unwrap().1[0].text, "Offline.\n");
    }
    #[test]
    fn concurrent_compactions_join_disjoint_edits_without_batches() {
        let mut a = document();
        let mut b = a.clone();
        for n in 0..201 {
            a.edit("a", "original", &format!("A {n}.\nTwo.\nThree."))
                .unwrap();
            b.edit("b", "original", &format!("One.\nTwo.\nB {n}."))
                .unwrap();
        }
        assert!(a.compact("original").unwrap());
        assert!(b.compact("original").unwrap());
        a.batches.clear();
        b.batches.clear();
        let before = a.clone();
        a.merge_immutable(&b).unwrap();
        b.merge_immutable(&before).unwrap();
        assert_eq!(a.text("original").unwrap(), "A 200.\nTwo.\nB 200.");
        assert_eq!(a.text("original"), b.text("original"));
    }
    #[test]
    fn synced_metadata_rejects_local_paths_and_unsafe_history_names() {
        let mut d = document();
        d.head
            .fields
            .insert("record.legacyAudio".into(), json!("C:/synthetic.wav"));
        assert!(d.validate().is_err());
        d.head.fields.clear();
        d.head
            .fields
            .insert("record.recognitionFile".into(), json!("../outside.json"));
        assert!(d.validate().is_err());
        d.head.fields.insert(
            "record.recognitionFile".into(),
            json!("recognition-one.json"),
        );
        assert!(d.validate().is_ok());
    }
    #[test]
    fn fields_keep_independent_edits_and_conflict_copies() {
        let mut conflicts = BTreeMap::new();
        let merged = merge_fields(
            &json!({"title":"one","speaker":"A"}),
            &json!({"title":"local","speaker":"A"}),
            &json!({"title":"remote","speaker":"B"}),
            "head",
            &mut conflicts,
        );
        assert_eq!(merged, json!({"title":"remote","speaker":"B"}));
        assert_eq!(conflicts["head/title"], "local");
    }
}
