//! CSS serialization and source map generation.

use crate::css_modules::CssModule;
use crate::dependencies::{Dependency, DependencyOptions};
use crate::error::{Error, ErrorLocation, PrinterError, PrinterErrorKind};
use crate::rules::{Location, StyleContext};
use crate::selector::SelectorList;
use crate::targets::{Targets, TargetsWithSupportsScope};
use crate::vendor_prefix::VendorPrefix;
use cssparser::{serialize_identifier, serialize_name};
#[cfg(feature = "sourcemap")]
use parcel_sourcemap::{OriginalLocation, SourceMap};
use std::fmt::Write;

/// Options that control how CSS is serialized to a string.
pub struct PrinterOptions<'a> {
  /// Whether to minify the CSS, i.e. remove white space.
  pub minify: bool,
  /// An optional reference to a source map to write mappings into.
  #[cfg(feature = "sourcemap")]
  #[cfg_attr(docsrs, doc(cfg(feature = "sourcemap")))]
  pub source_map: Option<&'a mut SourceMap>,
  /// An optional project root path, used to generate relative paths for sources used in CSS module hashes.
  pub project_root: Option<&'a str>,
  /// Targets to output the CSS for.
  pub targets: Targets,
  /// Whether to analyze dependencies (i.e. `@import` and `url()`).
  /// If true, the dependencies are returned as part of the
  /// [ToCssResult](super::stylesheet::ToCssResult).
  ///
  /// When enabled, `@import` and `url()` dependencies
  /// are replaced with hashed placeholders that can be replaced with the final
  /// urls later (after bundling).
  pub analyze_dependencies: Option<DependencyOptions>,
  /// A mapping of pseudo classes to replace with class names that can be applied
  /// from JavaScript. Useful for polyfills, for example.
  pub pseudo_classes: Option<PseudoClasses<'a>>,
}

impl<'a> Default for PrinterOptions<'a> {
  fn default() -> Self {
    PrinterOptions {
      minify: false,
      #[cfg(feature = "sourcemap")]
      source_map: None,
      project_root: None,
      targets: Targets::default(),
      analyze_dependencies: None,
      pseudo_classes: None,
    }
  }
}

pub(crate) struct PrinterState<'a, 'c> {
  pub(crate) loc: Location,
  pub(crate) generated_line: u32,
  pub(crate) generated_col: u32,
  pub(crate) indent: u8,
  pub(crate) targets: TargetsWithSupportsScope,
  pub(crate) vendor_prefix: VendorPrefix,
  pub(crate) in_calc: bool,
  pub(crate) css_module: Option<CssModule<'a, 'c>>,
  pub(crate) dependencies: Option<Vec<Dependency>>,
  pub(crate) remove_imports: bool,
  pub(crate) context: Option<&'a StyleContext<'a, 'c>>,
}

/// A mapping of user action pseudo classes to replace with class names.
///
/// See [PrinterOptions](PrinterOptions).
#[derive(Default, Debug)]
pub struct PseudoClasses<'a> {
  /// The class name to replace `:hover` with.
  pub hover: Option<&'a str>,
  /// The class name to replace `:active` with.
  pub active: Option<&'a str>,
  /// The class name to replace `:focus` with.
  pub focus: Option<&'a str>,
  /// The class name to replace `:focus-visible` with.
  pub focus_visible: Option<&'a str>,
  /// The class name to replace `:focus-within` with.
  pub focus_within: Option<&'a str>,
}

/// A `Printer` represents a destination to output serialized CSS, as used in
/// the [ToCss](super::traits::ToCss) trait. It can wrap any destination that
/// implements [std::fmt::Write](std::fmt::Write), such as a [String](String).
///
/// A `Printer` keeps track of the current line and column position, and uses
/// this to generate a source map if one is provided.
///
/// `Printer` also includes helper functions that assist with writing output
/// that respects options such as `minify`, and `css_modules`.
pub struct Printer<'a, 'c, W> {
  pub(crate) sources: Option<&'c Vec<String>>,
  dest: &'a mut W,
  pub(crate) options: PrinterOptions<'a>,
  pub(crate) state: PrinterState<'a, 'c>,
  #[cfg(feature = "sourcemap")]
  pub(crate) source_map: Option<&'a mut SourceMap>,
  #[cfg(feature = "sourcemap")]
  pub(crate) source_maps: Vec<Option<SourceMap>>,
  #[cfg(feature = "sourcemap")]
  pub(crate) source_map_source_indices: &'c [Option<u32>],
}

impl<'a, 'c, W: std::fmt::Write + Sized> Printer<'a, 'c, W> {
  /// Create a new Printer wrapping the given destination.
  #[cfg_attr(not(feature = "sourcemap"), allow(unused_mut))]
  pub fn new(dest: &'a mut W, mut options: PrinterOptions<'a>) -> Self {
    let dependencies = if options.analyze_dependencies.is_some() {
      Some(Vec::new())
    } else {
      None
    };
    let remove_imports = matches!(&options.analyze_dependencies, Some(d) if d.remove_imports);
    Printer {
      sources: None,
      dest,
      #[cfg(feature = "sourcemap")]
      source_map: options.source_map.take(),
      state: PrinterState {
        loc: Location {
          source_index: 0,
          line: 0,
          column: 1,
        },
        generated_line: 0,
        generated_col: 0,
        indent: 0,
        targets: TargetsWithSupportsScope::new(options.targets),
        vendor_prefix: VendorPrefix::empty(),
        in_calc: false,
        css_module: None,
        dependencies,
        remove_imports,
        context: None,
      },
      options,
      #[cfg(feature = "sourcemap")]
      source_maps: Vec::new(),
      #[cfg(feature = "sourcemap")]
      source_map_source_indices: &[],
    }
  }

  /// Returns the current source filename that is being printed.
  pub fn filename(&self) -> &'c str {
    if let Some(sources) = self.sources {
      if let Some(f) = sources.get(self.state.loc.source_index as usize) {
        f
      } else {
        "unknown.css"
      }
    } else {
      "unknown.css"
    }
  }

  /// Writes a single whitespace character, unless the `minify` option is enabled.
  ///
  /// Use `write_char` instead if you wish to force a space character to be written,
  /// regardless of the `minify` option.
  pub fn whitespace(&mut self) -> Result<(), PrinterError> {
    if self.options.minify {
      return Ok(());
    }

    self.write_char(' ')?;
    Ok(())
  }

  /// Writes a delimiter character, followed by whitespace (depending on the `minify` option).
  /// If `ws_before` is true, then whitespace is also written before the delimiter.
  pub fn delim(&mut self, delim: char, ws_before: bool) -> Result<(), PrinterError> {
    if ws_before {
      self.whitespace()?;
    }
    self.write_char(delim)?;
    self.whitespace()
  }

  /// Writes a newline character followed by indentation.
  /// If the `minify` option is enabled, then nothing is printed.
  pub fn newline(&mut self) -> Result<(), PrinterError> {
    if self.options.minify {
      return Ok(());
    }

    self.write_char('\n')?;
    if self.state.indent > 0 {
      self.write_str(&" ".repeat(self.state.indent as usize))?;
    }

    Ok(())
  }

  /// Increases the current indent level.
  pub fn indent(&mut self) {
    self.state.indent += 2;
  }

  /// Decreases the current indent level.
  pub fn dedent(&mut self) {
    self.state.indent -= 2;
  }

  /// Increases the current indent level by the given number of characters.
  pub fn indent_by(&mut self, amt: u8) {
    self.state.indent += amt;
  }

  /// Decreases the current indent level by the given number of characters.
  pub fn dedent_by(&mut self, amt: u8) {
    self.state.indent -= amt;
  }

  /// Returns whether the indent level is greater than one.
  pub fn is_nested(&self) -> bool {
    self.state.indent > 2
  }

  /// Writes a CSS identifier to the underlying destination, escaping it
  /// as appropriate. If the `css_modules` option was enabled, then a hash
  /// is added, and the mapping is added to the CSS module.
  pub fn write_ident(&mut self, ident: &str, handle_css_module: bool) -> Result<(), PrinterError> {
    if handle_css_module {
      if let Some(css_module) = &mut self.state.css_module {
        let dest = &mut self.dest;
        let mut first = true;
        css_module.config.pattern.write(
          &css_module.hashes[self.state.loc.source_index as usize],
          &css_module.sources[self.state.loc.source_index as usize],
          ident,
          if let Some(content_hashes) = &css_module.content_hashes {
            &content_hashes[self.state.loc.source_index as usize]
          } else {
            ""
          },
          |s| {
            self.state.generated_col += s.len() as u32;
            if first {
              first = false;
              serialize_identifier(s, dest)
            } else {
              serialize_name(s, dest)
            }
          },
        )?;

        css_module.add_local(&ident, &ident, self.state.loc.source_index);
        return Ok(());
      }
    }

    serialize_identifier(ident, self)?;
    Ok(())
  }

  pub(crate) fn write_dashed_ident(&mut self, ident: &str, is_declaration: bool) -> Result<(), PrinterError> {
    self.write_str("--")?;

    match &mut self.state.css_module {
      Some(css_module) if css_module.config.dashed_idents => {
        let dest = &mut self.dest;
        css_module.config.pattern.write(
          &css_module.hashes[self.state.loc.source_index as usize],
          &css_module.sources[self.state.loc.source_index as usize],
          &ident[2..],
          if let Some(content_hashes) = &css_module.content_hashes {
            &content_hashes[self.state.loc.source_index as usize]
          } else {
            ""
          },
          |s| {
            self.state.generated_col += s.len() as u32;
            serialize_name(s, dest)
          },
        )?;

        if is_declaration {
          css_module.add_dashed(ident, self.state.loc.source_index);
        }
      }
      _ => {
        serialize_name(&ident[2..], self)?;
      }
    }

    Ok(())
  }

  /// Returns an error of the given kind at the provided location in the current source file.
  pub fn error(&self, kind: PrinterErrorKind, loc: crate::dependencies::Location) -> Error<PrinterErrorKind> {
    Error {
      kind,
      loc: Some(ErrorLocation {
        filename: self.filename().into(),
        line: loc.line - 1,
        column: loc.column,
      }),
    }
  }

  pub(crate) fn take_dependencies(&mut self) -> Option<Vec<Dependency>> {
    self.state.dependencies.take()
  }

  /// Adds a mapping to the source map, if any.
  #[cfg(feature = "sourcemap")]
  #[cfg_attr(docsrs, doc(cfg(feature = "sourcemap")))]
  pub fn add_mapping(&mut self, loc: Location) {
    self.state.loc = loc;

    if let Some(map) = &mut self.source_map {
      let mut original = OriginalLocation {
        original_line: loc.line,
        original_column: loc.column - 1,
        source: self
          .source_map_source_indices
          .get(loc.source_index as usize)
          .copied()
          .flatten()
          .unwrap_or(loc.source_index),
        name: None,
      };

      // Remap using input source map if possible.
      if let Some(Some(sm)) = self.source_maps.get_mut(loc.source_index as usize) {
        let mut found_mapping = false;
        if let Some(mapping) = sm.find_closest_mapping(loc.line, loc.column - 1) {
          if let Some(orig) = mapping.original {
            let sources_len = map.get_sources().len();
            let source_index = map.add_source(sm.get_source(orig.source).unwrap());
            let name = orig.name.map(|name| map.add_name(sm.get_name(name).unwrap()));
            original.original_line = orig.original_line;
            original.original_column = orig.original_column;
            original.source = source_index;
            original.name = name;

            if map.get_sources().len() > sources_len {
              let content = sm.get_source_content(orig.source).unwrap().to_owned();
              let _ = map.set_source_content(source_index as usize, &content);
            }

            found_mapping = true;
          }
        }

        if !found_mapping {
          return;
        }
      }

      map.add_mapping(self.state.generated_line, self.state.generated_col, Some(original));
    }
  }

  fn with_context<T>(&mut self, selectors: &SelectorList, f: impl FnOnce(&mut Self) -> T) -> T {
    let parent = self.state.context.take();
    let ctx = StyleContext {
      selectors: unsafe { std::mem::transmute(selectors) },
      parent,
    };

    // The context is only installed for the duration of the callback.
    self.state.context = Some(unsafe { std::mem::transmute(&ctx) });
    let res = f(self);
    self.state.context = parent;
    res
  }

  fn with_cleared_context<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
    let parent = self.state.context.take();
    let res = f(self);
    self.state.context = parent;
    res
  }

  fn with_parent_context<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
    let parent = self.state.context.take();
    if let Some(parent) = parent {
      self.state.context = parent.parent;
    }
    let res = f(self);
    self.state.context = parent;
    res
  }
}

impl<'a, 'c, W: std::fmt::Write + Sized> Write for Printer<'a, 'c, W> {
  fn write_str(&mut self, s: &str) -> std::fmt::Result {
    let mut last_line_start = 0;
    for (idx, c) in s.char_indices() {
      if c == '\n' {
        self.state.generated_line += 1;
        last_line_start = idx + 1;
      }
    }

    self.state.generated_col = if last_line_start == 0 {
      self.state.generated_col + s.len() as u32
    } else {
      (s.len() - last_line_start) as u32
    };
    self.dest.write_str(s)
  }

  fn write_char(&mut self, c: char) -> std::fmt::Result {
    if c == '\n' {
      self.state.generated_line += 1;
      self.state.generated_col = 0;
    } else {
      self.state.generated_col += 1;
    }

    self.dest.write_char(c)
  }
}

// Supertrait methods can be called through a public generic bound. Require a
// crate-local token to run a callback with an installed style context.
pub(crate) struct ContextAccess;

mod private {
  use super::*;

  pub(crate) trait Sealed {
    fn targets(&self) -> &TargetsWithSupportsScope;
    fn targets_mut(&mut self) -> &mut TargetsWithSupportsScope;
    fn vendor_prefix(&self) -> VendorPrefix;
    fn set_vendor_prefix(&mut self, prefix: VendorPrefix);
    fn loc(&self) -> Location;
    fn in_calc(&self) -> bool;
    fn set_in_calc(&mut self, in_calc: bool);
    fn css_module_config(&self) -> Option<&crate::css_modules::Config>;
    fn with_css_module_mut<T>(&mut self, f: impl FnOnce(&mut CssModule<'_, '_>) -> T) -> Option<T>;
    fn is_nested(&self) -> bool;
    fn has_dependencies(&self) -> bool;
    fn add_dependency(&mut self, dep: Dependency);
    fn remove_imports(&self) -> bool;
    fn has_context(&self) -> bool;
    fn with_context<T>(
      &mut self,
      access: ContextAccess,
      selectors: &SelectorList,
      f: impl FnOnce(&mut Self) -> T,
    ) -> T;
    fn with_cleared_context<T>(&mut self, access: ContextAccess, f: impl FnOnce(&mut Self) -> T) -> T;
    fn with_parent_context<T>(&mut self, access: ContextAccess, f: impl FnOnce(&mut Self) -> T) -> T;
    fn with_current_context<T>(
      &mut self,
      access: ContextAccess,
      f: impl FnOnce(&mut Self, Option<&StyleContext<'_, '_>>) -> T,
    ) -> T;
    fn with_pseudo_class<T>(
      &mut self,
      get: impl for<'p> FnOnce(&PseudoClasses<'p>) -> Option<&'p str>,
      f: impl FnOnce(&mut Self, Option<&str>) -> T,
    ) -> T;
  }
}

/// A printer abstraction used by CSS serialization implementations.
#[allow(missing_docs)]
#[allow(private_bounds)]
pub trait PrinterTrait: std::fmt::Write + private::Sealed {
  fn options(&self) -> &PrinterOptions<'_>;
  fn filename(&self) -> &str;
  fn error(&self, kind: PrinterErrorKind, loc: crate::dependencies::Location) -> Error<PrinterErrorKind> {
    Error {
      kind,
      loc: Some(ErrorLocation {
        filename: self.filename().into(),
        line: loc.line - 1,
        column: loc.column,
      }),
    }
  }

  fn whitespace(&mut self) -> Result<(), PrinterError>;
  fn delim(&mut self, c: char, ws_before: bool) -> Result<(), PrinterError>;
  fn newline(&mut self) -> Result<(), PrinterError>;
  #[cfg(feature = "sourcemap")]
  fn add_mapping(&mut self, loc: Location);
  fn indent(&mut self);
  fn dedent(&mut self);
  fn indent_by(&mut self, amt: u8);
  fn dedent_by(&mut self, amt: u8);
  fn write_ident(&mut self, ident: &str, handle_css_module: bool) -> Result<(), PrinterError>;
  fn write_dashed_ident(&mut self, ident: &str, is_declaration: bool) -> Result<(), PrinterError>;

  fn without_css_module<T, E>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, E>) -> Result<T, E>;
}

impl<W: Write> private::Sealed for Printer<'_, '_, W> {
  fn targets(&self) -> &TargetsWithSupportsScope {
    &self.state.targets
  }

  fn targets_mut(&mut self) -> &mut TargetsWithSupportsScope {
    &mut self.state.targets
  }

  fn vendor_prefix(&self) -> VendorPrefix {
    self.state.vendor_prefix
  }

  fn set_vendor_prefix(&mut self, prefix: VendorPrefix) {
    self.state.vendor_prefix = prefix;
  }

  fn loc(&self) -> Location {
    self.state.loc
  }

  fn in_calc(&self) -> bool {
    self.state.in_calc
  }

  fn set_in_calc(&mut self, in_calc: bool) {
    self.state.in_calc = in_calc;
  }

  fn css_module_config(&self) -> Option<&crate::css_modules::Config> {
    self.state.css_module.as_ref().map(|css_module| css_module.config)
  }

  fn with_css_module_mut<T>(&mut self, f: impl FnOnce(&mut CssModule<'_, '_>) -> T) -> Option<T> {
    self.state.css_module.as_mut().map(f)
  }

  fn is_nested(&self) -> bool {
    Printer::is_nested(self)
  }

  fn has_dependencies(&self) -> bool {
    self.state.dependencies.is_some()
  }

  fn add_dependency(&mut self, dep: Dependency) {
    if let Some(dependencies) = &mut self.state.dependencies {
      dependencies.push(dep);
    }
  }

  fn remove_imports(&self) -> bool {
    self.state.remove_imports
  }

  fn has_context(&self) -> bool {
    self.state.context.is_some()
  }

  fn with_context<T>(&mut self, _: ContextAccess, selectors: &SelectorList, f: impl FnOnce(&mut Self) -> T) -> T {
    Printer::with_context(self, selectors, f)
  }

  fn with_cleared_context<T>(&mut self, _: ContextAccess, f: impl FnOnce(&mut Self) -> T) -> T {
    Printer::with_cleared_context(self, f)
  }

  fn with_parent_context<T>(&mut self, _: ContextAccess, f: impl FnOnce(&mut Self) -> T) -> T {
    Printer::with_parent_context(self, f)
  }

  fn with_current_context<T>(
    &mut self,
    _: ContextAccess,
    f: impl FnOnce(&mut Self, Option<&StyleContext<'_, '_>>) -> T,
  ) -> T {
    let context = self.state.context;
    f(self, context)
  }

  fn with_pseudo_class<T>(
    &mut self,
    get: impl for<'p> FnOnce(&PseudoClasses<'p>) -> Option<&'p str>,
    f: impl FnOnce(&mut Self, Option<&str>) -> T,
  ) -> T {
    let class = self.options.pseudo_classes.as_ref().and_then(get);
    f(self, class)
  }
}

impl<'a, 'c, W: std::fmt::Write + Sized> PrinterTrait for Printer<'a, 'c, W> {
  fn options(&self) -> &PrinterOptions<'_> {
    &self.options
  }

  fn filename(&self) -> &str {
    Printer::filename(self)
  }

  fn whitespace(&mut self) -> Result<(), PrinterError> {
    Printer::whitespace(self)
  }

  fn delim(&mut self, c: char, ws_before: bool) -> Result<(), PrinterError> {
    Printer::delim(self, c, ws_before)
  }

  fn newline(&mut self) -> Result<(), PrinterError> {
    Printer::newline(self)
  }

  #[cfg(feature = "sourcemap")]
  fn add_mapping(&mut self, loc: Location) {
    Printer::add_mapping(self, loc)
  }

  fn indent(&mut self) {
    Printer::indent(self)
  }

  fn dedent(&mut self) {
    Printer::dedent(self)
  }

  fn indent_by(&mut self, amt: u8) {
    Printer::indent_by(self, amt)
  }

  fn dedent_by(&mut self, amt: u8) {
    Printer::dedent_by(self, amt)
  }

  fn write_ident(&mut self, ident: &str, handle_css_module: bool) -> Result<(), PrinterError> {
    Printer::write_ident(self, ident, handle_css_module)
  }

  fn write_dashed_ident(&mut self, ident: &str, is_declaration: bool) -> Result<(), PrinterError> {
    Printer::write_dashed_ident(self, ident, is_declaration)
  }

  fn without_css_module<T, E>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, E>) -> Result<T, E> {
    let css_module = self.state.css_module.take();
    let res = f(self)?;
    self.state.css_module = css_module;
    Ok(res)
  }
}

#[cfg(test)]
mod tests {
  use super::{Printer, PrinterOptions};
  use std::fmt::Write;

  #[test]
  fn write_tracks_multiline_positions() {
    let mut output = String::new();
    let mut printer = Printer::new(&mut output, PrinterOptions::default());

    printer.write_str("a\nbc").unwrap();
    printer.write_char('\n').unwrap();
    printer.write_str("d").unwrap();

    assert_eq!((printer.state.generated_line, printer.state.generated_col), (2, 1));
    drop(printer);
    assert_eq!(output, "a\nbc\nd");
  }
}
