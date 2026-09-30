use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result, bail};
use axum::body::Body;
use futures_util::StreamExt;
use serde_json::Value;
use serde_json::ser::{CompactFormatter, Formatter};
use struson::{
    reader::{JsonReader, JsonStreamReader, ValueType},
    writer::{JsonStreamWriter, JsonWriter, StringValueWriter},
};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::server::BodyProcessing;

type Reader = JsonStreamReader<BufReader<io::Take<FileCursor>>>;

struct FileCursor {
    file: Arc<File>,
    offset: u64,
}

impl Read for FileCursor {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        #[cfg(unix)]
        let count = {
            use std::os::unix::fs::FileExt;
            self.file.read_at(buffer, self.offset)?
        };
        #[cfg(windows)]
        let count = {
            use std::os::windows::fs::FileExt;
            self.file.seek_read(buffer, self.offset)?
        };
        self.offset += count as u64;
        Ok(count)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Span {
    start: u64,
    len: u64,
}

struct TrackedWriter {
    inner: BufWriter<File>,
    position: u64,
    root_fields: BTreeMap<String, Span>,
}

impl Write for TrackedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(bytes)?;
        self.position += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[derive(Clone)]
pub struct JsonBody {
    file: Arc<File>,
    pub len: u64,
    settings: BodyProcessing,
    root_fields: Arc<OnceLock<BTreeMap<String, Span>>>,
}

#[derive(Clone)]
pub enum Replacement {
    Json(Value),
    Source(JsonBody, Span),
}

#[derive(Default)]
pub struct Patch {
    pub remove: Vec<Vec<String>>,
    pub set: BTreeMap<Vec<String>, Replacement>,
    pub retain: Option<Vec<String>>,
    pub drop_null: bool,
    pub text: Vec<(Vec<String>, String, String)>,
    pub canonical: bool,
    pub role_map: BTreeMap<String, String>,
    pub tool_defaults: Option<crate::runtime::ToolDefaults>,
    pub object_required: bool,
}

pub fn pointer(path: &str) -> Vec<String> {
    path.split('/')
        .skip(1)
        .map(|part| part.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn matches(pattern: &[String], path: &[String]) -> bool {
    pattern.len() == path.len() && pattern.iter().zip(path).all(|(a, b)| a == "*" || a == b)
}

fn position(reader: &Reader) -> u64 {
    reader
        .current_position(false)
        .data_pos
        .expect("file reader byte position")
}

fn take_span(reader: &mut Reader, base: u64) -> Result<Span> {
    reader.peek()?;
    let start = position(reader);
    reader.skip_value()?;
    Ok(Span {
        start: base + start,
        len: position(reader) - start,
    })
}

impl JsonBody {
    fn empty(settings: &BodyProcessing) -> Result<Self> {
        let file = tempfile::tempfile_in(&settings.spool_directory)?;
        Ok(Self {
            file: Arc::new(file),
            len: 0,
            settings: settings.clone(),
            root_fields: Arc::new(OnceLock::new()),
        })
    }

    pub async fn receive(body: Body, settings: &BodyProcessing, max: usize) -> Result<Self> {
        let mut stored = Self::empty(settings)?;
        let mut file = tokio::fs::File::from_std(stored.file.try_clone()?);
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            stored.len = stored
                .len
                .checked_add(chunk.len() as u64)
                .context("request length overflow")?;
            if stored.len > max as u64 {
                bail!("request exceeds configured byte limit");
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(stored)
    }

    pub fn root(&self) -> Span {
        Span {
            start: 0,
            len: self.len,
        }
    }

    fn reader(&self, span: Span) -> Result<Reader> {
        let file = FileCursor {
            file: self.file.clone(),
            offset: span.start,
        };
        Ok(JsonStreamReader::new(BufReader::with_capacity(
            self.settings.io_buffer_bytes,
            file.take(span.len),
        )))
    }

    pub fn fields(&self, span: Span) -> Result<BTreeMap<String, Span>> {
        let root = span.start == 0 && span.len == self.len;
        if root && let Some(fields) = self.root_fields.get() {
            return Ok(fields.clone());
        }
        let mut reader = self.reader(span)?;
        reader.begin_object()?;
        let mut result = BTreeMap::new();
        while reader.has_next()? {
            let name = reader.next_name_owned()?;
            result.insert(name, take_span(&mut reader, span.start)?);
        }
        reader.end_object()?;
        reader.consume_trailing_whitespace()?;
        if root {
            let _ = self.root_fields.set(result.clone());
        }
        Ok(result)
    }

    pub fn at(&self, path: &[String]) -> Result<Option<Span>> {
        let mut span = self.root();
        for token in path {
            let mut reader = self.reader(span)?;
            match reader.peek()? {
                ValueType::Object => {
                    let Some(next) = self.fields(span)?.get(token).copied() else {
                        return Ok(None);
                    };
                    span = next;
                }
                ValueType::Array => {
                    let Ok(index) = token.parse::<usize>() else {
                        return Ok(None);
                    };
                    reader.begin_array()?;
                    let mut next = None;
                    let mut current = 0;
                    while reader.has_next()? {
                        if current == index {
                            next = Some(take_span(&mut reader, span.start)?);
                            break;
                        }
                        reader.skip_value()?;
                        current += 1;
                    }
                    let Some(next) = next else {
                        return Ok(None);
                    };
                    span = next;
                }
                _ => return Ok(None),
            }
        }
        Ok(Some(span))
    }

    pub fn value(&self, span: Span) -> Result<Value> {
        if span.len > self.settings.metadata_limit_bytes as u64 {
            bail!("routing metadata exceeds configured metadata_limit_bytes");
        }
        Ok(self.reader(span)?.deserialize_next()?)
    }

    pub fn value_at(&self, path: &str) -> Result<Option<Value>> {
        self.at(&pointer(path))?
            .map(|span| self.value(span))
            .transpose()
    }

    pub fn kind(&self, span: Span) -> Result<ValueType> {
        Ok(self.reader(span)?.peek()?)
    }

    pub fn visit_array(
        &self,
        path: &str,
        mut visit: impl FnMut(usize, Span) -> Result<()>,
    ) -> Result<()> {
        let Some(span) = self.at(&pointer(path))? else {
            return Ok(());
        };
        let mut reader = self.reader(span)?;
        if reader.peek()? != ValueType::Array {
            return Ok(());
        }
        reader.begin_array()?;
        let mut index = 0;
        while reader.has_next()? {
            visit(index, take_span(&mut reader, span.start)?)?;
            index += 1;
        }
        reader.end_array()?;
        Ok(())
    }

    pub fn last_user_matches(&self, markers: &[String]) -> Result<Vec<String>> {
        if markers.is_empty() {
            return Ok(Vec::new());
        }
        let Some(messages) = self.at(&pointer("/messages"))? else {
            return Ok(Vec::new());
        };
        let mut reader = self.reader(messages)?;
        if reader.peek()? != ValueType::Array {
            return Ok(Vec::new());
        }
        reader.begin_array()?;
        let mut last = None;
        while reader.has_next()? {
            let message = take_span(&mut reader, messages.start)?;
            if self.kind(message)? != ValueType::Object {
                continue;
            }
            let fields = self.fields(message)?;
            if let Some(role) = fields.get("role")
                && self.value(*role)?.as_str() == Some("user")
                && let Some(content) = fields.get("content")
            {
                last = Some(*content);
            }
        }
        let Some(last) = last else {
            return Ok(Vec::new());
        };
        let mut found = vec![false; markers.len()];
        self.scan_strings(&mut self.reader(last)?, markers, &mut found)?;
        Ok(markers
            .iter()
            .zip(found)
            .filter(|(_, found)| *found)
            .map(|(marker, _)| marker.clone())
            .collect())
    }

    fn scan_strings(
        &self,
        reader: &mut Reader,
        markers: &[String],
        found: &mut [bool],
    ) -> Result<()> {
        match reader.peek()? {
            ValueType::Array => {
                reader.begin_array()?;
                while reader.has_next()? {
                    self.scan_strings(reader, markers, found)?;
                }
                reader.end_array()?;
            }
            ValueType::Object => {
                reader.begin_object()?;
                while reader.has_next()? {
                    reader.next_name()?;
                    self.scan_strings(reader, markers, found)?;
                }
                reader.end_object()?;
            }
            ValueType::String => {
                let mut value = reader.next_string_reader()?;
                let overlap = markers.iter().map(String::len).max().unwrap_or(0);
                let mut window = Vec::with_capacity(self.settings.io_buffer_bytes + overlap);
                let mut buffer = vec![0; self.settings.io_buffer_bytes];
                loop {
                    let n = value.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    window.extend_from_slice(&buffer[..n]);
                    for (marker, hit) in markers.iter().zip(found.iter_mut()) {
                        if !*hit && memchr::memmem::find(&window, marker.as_bytes()).is_some() {
                            *hit = true;
                        }
                    }
                    let keep = window.len().saturating_sub(overlap);
                    window.drain(..keep);
                }
            }
            _ => reader.skip_value()?,
        }
        Ok(())
    }

    pub fn rewrite(&self, patch: &Patch) -> Result<Self> {
        let mut output = Self::empty(&self.settings)?;
        let file = output.file.try_clone()?;
        let mut writer = TrackedWriter {
            inner: BufWriter::with_capacity(self.settings.io_buffer_bytes, file),
            position: 0,
            root_fields: BTreeMap::new(),
        };
        self.write_value(self.root(), &mut Vec::new(), patch, &mut writer)?;
        writer.flush()?;
        output.len = writer.position;
        if !writer.root_fields.is_empty() {
            let _ = output.root_fields.set(writer.root_fields);
        }
        Ok(output)
    }

    fn copy_span(&self, span: Span, writer: &mut impl Write) -> Result<()> {
        let source = FileCursor {
            file: self.file.clone(),
            offset: span.start,
        }
        .take(span.len);
        io::copy(
            &mut BufReader::with_capacity(self.settings.io_buffer_bytes, source),
            writer,
        )?;
        Ok(())
    }

    fn write_replacement<W: Write>(&self, replacement: &Replacement, writer: &mut W) -> Result<()> {
        match replacement {
            Replacement::Json(value) => serde_json::to_writer(writer, value)?,
            Replacement::Source(body, span) => body.copy_span(*span, writer)?,
        }
        Ok(())
    }

    fn write_value(
        &self,
        span: Span,
        path: &mut Vec<String>,
        patch: &Patch,
        writer: &mut TrackedWriter,
    ) -> Result<()> {
        if let Some(value) = patch.set.get(path) {
            return self.write_replacement(value, writer);
        }
        let mut reader = self.reader(span)?;
        let kind = reader.peek()?;
        if kind != ValueType::Object
            && kind != ValueType::Array
            && patch
                .set
                .keys()
                .any(|target| target.len() > path.len() && target.starts_with(path))
        {
            bail!("JSON Pointer parent is not an object or array");
        }
        let text_rules = patch
            .text
            .iter()
            .filter(|(pattern, _, _)| matches(pattern, path))
            .collect::<Vec<_>>();
        if !text_rules.is_empty() && kind == ValueType::String {
            let mut temporary = tempfile::tempfile_in(&self.settings.spool_directory)?;
            io::copy(&mut reader.next_string_reader()?, &mut temporary)?;
            for (_, prefix, replacement) in text_rules {
                temporary.seek(SeekFrom::Start(0))?;
                let mut next = tempfile::tempfile_in(&self.settings.spool_directory)?;
                replace_lines(
                    &mut temporary,
                    &mut next,
                    prefix,
                    replacement,
                    self.settings.io_buffer_bytes,
                )?;
                temporary = next;
            }
            temporary.seek(SeekFrom::Start(0))?;
            let mut json_writer = JsonStreamWriter::new(writer);
            let mut output = json_writer.string_value_writer()?;
            io::copy(&mut temporary, &mut output)?;
            output.finish_value()?;
            json_writer.finish_document_no_flush()?;
            return Ok(());
        }
        let relevant = |candidate: &[String]| {
            candidate.len() > path.len()
                && candidate
                    .iter()
                    .zip(path.iter())
                    .all(|(a, b)| a == "*" || a == b)
        };
        let inspect = patch.canonical
            || patch.drop_null
            || patch.object_required
            || path.is_empty() && patch.retain.is_some()
            || patch.remove.iter().any(|p| relevant(p))
            || patch.set.keys().any(|p| relevant(p))
            || patch.text.iter().any(|(p, _, _)| relevant(p))
            || patch.tool_defaults.is_some() && (path.is_empty() || path[0] == "tools")
            || !patch.role_map.is_empty() && (path.is_empty() || path[0] == "messages");
        if !inspect {
            self.copy_span(span, writer)?;
            return Ok(());
        }
        if path.len() == 3
            && path[0] == "messages"
            && path[2] == "role"
            && !patch.role_map.is_empty()
        {
            let value = self.value(span)?;
            if let Some(mapped) = value.as_str().and_then(|role| patch.role_map.get(role)) {
                serde_json::to_writer(writer, mapped)?;
                return Ok(());
            }
        }
        match kind {
            ValueType::Object => {
                let fields = self.fields(span)?;
                let mut additions = BTreeMap::new();
                for (target, value) in &patch.set {
                    if target.len() > path.len() && target.starts_with(path) {
                        additions.insert(target[path.len()].clone(), value);
                    }
                }
                let mut defaults = BTreeMap::new();
                if patch.object_required
                    && fields.contains_key("properties")
                    && let Some(type_span) = fields.get("type")
                    && self.value(*type_span)?.as_str() == Some("object")
                    && fields
                        .get("required")
                        .map(|span| self.kind(*span))
                        .transpose()?
                        != Some(ValueType::Array)
                {
                    defaults.insert("required".to_owned(), Value::Array(Vec::new()));
                }
                if path.len() == 2
                    && path[0] == "tools"
                    && let Some(policy) = &patch.tool_defaults
                    && let Some(name) = fields.get("name")
                {
                    let name = self.value(*name)?;
                    if let Some(name) = name.as_str().filter(|name| !name.trim().is_empty()) {
                        let description = fields
                            .get("description")
                            .map(|span| self.value(*span))
                            .transpose()?;
                        if description
                            .as_ref()
                            .and_then(Value::as_str)
                            .is_none_or(|value| value.trim().is_empty())
                        {
                            defaults.insert(
                                "description".to_owned(),
                                Value::String(policy.description_template.replace("{name}", name)),
                            );
                        }
                        if fields
                            .get("input_schema")
                            .map(|span| self.kind(*span))
                            .transpose()?
                            != Some(ValueType::Object)
                        {
                            let mut schema = policy.input_schema.clone();
                            if patch.object_required {
                                crate::normalize_object_schema(&mut schema);
                            }
                            defaults.insert("input_schema".to_owned(), schema);
                        }
                    }
                }
                let keys = fields
                    .keys()
                    .chain(additions.keys())
                    .chain(defaults.keys())
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                CompactFormatter.begin_object(writer)?;
                let mut first = true;
                for key in keys {
                    path.push(key.clone());
                    let removed = patch.remove.iter().any(|pattern| matches(pattern, path))
                        || (path.len() == 1
                            && patch
                                .retain
                                .as_ref()
                                .is_some_and(|keys| !keys.contains(&key)));
                    let original = fields.get(&key).copied();
                    let null = patch.drop_null
                        && original.map(|span| self.kind(span)).transpose()?
                            == Some(ValueType::Null);
                    let mut start = None;
                    if let Some(value) = patch.set.get(path) {
                        write_name(writer, &key, &mut first)?;
                        start = Some(writer.position);
                        self.write_replacement(value, writer)?;
                    } else if let Some(value) = defaults.get(&key) {
                        write_name(writer, &key, &mut first)?;
                        start = Some(writer.position);
                        serde_json::to_writer(&mut *writer, value)?;
                    } else if (!removed && !null && original.is_some())
                        || additions.contains_key(&key)
                    {
                        write_name(writer, &key, &mut first)?;
                        start = Some(writer.position);
                        if let Some(original) = original.filter(|_| !removed && !null) {
                            self.write_value(original, path, patch, writer)?;
                        } else {
                            self.write_additions(path, patch, writer)?;
                        }
                    }
                    if path.len() == 1
                        && let Some(start) = start
                    {
                        writer.root_fields.insert(
                            key,
                            Span {
                                start,
                                len: writer.position - start,
                            },
                        );
                    }
                    path.pop();
                }
                CompactFormatter.end_object(writer)?;
            }
            ValueType::Array => {
                reader.begin_array()?;
                CompactFormatter.begin_array(writer)?;
                let mut index = 0;
                while reader.has_next()? {
                    let child = take_span(&mut reader, span.start)?;
                    CompactFormatter.begin_array_value(writer, index == 0)?;
                    path.push(index.to_string());
                    self.write_value(child, path, patch, writer)?;
                    path.pop();
                    index += 1;
                }
                reader.end_array()?;
                CompactFormatter.end_array(writer)?;
            }
            _ => {
                if patch.canonical && kind != ValueType::String {
                    serde_json::to_writer(writer, &self.value(span)?)?;
                } else if patch.canonical {
                    let mut json_writer = JsonStreamWriter::new(writer);
                    reader.transfer_to(&mut json_writer)?;
                    json_writer.finish_document_no_flush()?;
                } else {
                    self.copy_span(span, writer)?;
                }
            }
        }
        Ok(())
    }

    fn write_additions(
        &self,
        path: &mut Vec<String>,
        patch: &Patch,
        writer: &mut impl Write,
    ) -> Result<()> {
        if let Some(value) = patch.set.get(path) {
            return self.write_replacement(value, writer);
        }
        CompactFormatter.begin_object(writer)?;
        let keys = patch
            .set
            .keys()
            .filter(|target| target.starts_with(path) && target.len() > path.len())
            .map(|target| target[path.len()].clone())
            .collect::<std::collections::BTreeSet<_>>();
        let mut first = true;
        for key in keys {
            write_name(writer, &key, &mut first)?;
            path.push(key);
            self.write_additions(path, patch, writer)?;
            path.pop();
        }
        CompactFormatter.end_object(writer)?;
        Ok(())
    }

    pub async fn into_http_body(self) -> Result<reqwest::Body> {
        let mut source = self.file.try_clone()?;
        source.seek(SeekFrom::Start(0))?;
        let file = tokio::fs::File::from_std(source);
        let keep_alive = self.file.clone();
        let stream =
            ReaderStream::with_capacity(file, self.settings.io_buffer_bytes).map(move |chunk| {
                let _ = &keep_alive;
                chunk
            });
        Ok(reqwest::Body::wrap_stream(stream))
    }

    #[cfg(test)]
    pub fn bytes_for_verification(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        FileCursor {
            file: self.file.clone(),
            offset: 0,
        }
        .read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

fn write_name(writer: &mut impl Write, name: &str, first: &mut bool) -> Result<()> {
    CompactFormatter.begin_object_key(writer, *first)?;
    *first = false;
    serde_json::to_writer(&mut *writer, name)?;
    CompactFormatter.end_object_key(writer)?;
    CompactFormatter.begin_object_value(writer)?;
    Ok(())
}

fn replace_lines(
    source: impl Read,
    output: &mut impl Write,
    prefix: &str,
    replacement: &str,
    buffer_size: usize,
) -> Result<()> {
    let mut source = BufReader::with_capacity(buffer_size, source);
    loop {
        if source.fill_buf()?.is_empty() {
            break;
        }
        let mut head = Vec::with_capacity(prefix.len());
        while head.len() < prefix.len() {
            let chunk = source.fill_buf()?;
            if chunk.is_empty() {
                break;
            }
            let n = chunk
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(chunk.len(), |n| n + 1)
                .min(prefix.len() - head.len());
            head.extend_from_slice(&chunk[..n]);
            source.consume(n);
            if head.last() == Some(&b'\n') {
                break;
            }
        }
        let replace = head == prefix.as_bytes();
        if replace {
            output.write_all(replacement.as_bytes())?;
        } else {
            output.write_all(&head)?;
        }
        if head.last() == Some(&b'\n') {
            continue;
        }
        let mut previous = head.last().copied();
        loop {
            let chunk = source.fill_buf()?;
            if chunk.is_empty() {
                break;
            }
            let end = chunk.iter().position(|byte| *byte == b'\n');
            let n = end.map_or(chunk.len(), |index| index + 1);
            if replace {
                if let Some(index) = end {
                    let cr = if index == 0 {
                        previous == Some(b'\r')
                    } else {
                        chunk[index - 1] == b'\r'
                    };
                    output.write_all(if cr { b"\r\n" } else { b"\n" })?;
                }
            } else {
                output.write_all(&chunk[..n])?;
            }
            previous = chunk.get(n - 1).copied();
            source.consume(n);
            if end.is_some() {
                break;
            }
        }
    }
    Ok(())
}
