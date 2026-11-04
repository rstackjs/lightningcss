#![cfg(all(feature = "bundler", feature = "sourcemap"))]

use parcel_sourcemap::SourceMap;
use rspack_lightningcss::{
  bundler::{Bundler, ResolveResult, SourceProvider},
  stylesheet::{ParserOptions, PrinterOptions},
};
use std::{io, path::Path};

struct SingleFile(&'static str);

impl SourceProvider for SingleFile {
  type Error = io::Error;

  fn read<'a>(&'a self, file: &Path) -> Result<&'a str, Self::Error> {
    if file == Path::new("/entry.css") {
      Ok(self.0)
    } else {
      Err(io::ErrorKind::NotFound.into())
    }
  }

  fn resolve(&self, _: &str, _: &Path) -> Result<ResolveResult, Self::Error> {
    Err(io::ErrorKind::NotFound.into())
  }
}

fn assert_bundled_source(css: &'static str) {
  let fs = SingleFile(css);
  let mut map = SourceMap::new("/");
  let existing_index = map.add_source("/existing.css");
  map.set_source_content(existing_index as usize, ".existing {}").unwrap();

  let mut bundler = Bundler::new(&fs, Some(&mut map), ParserOptions::default());
  let stylesheet = bundler.bundle(Path::new("/entry.css")).unwrap();
  stylesheet.to_css(PrinterOptions::default().with_source_map(&mut map)).unwrap();

  let source_index = map.find_closest_mapping(0, 0).unwrap().original.unwrap().source;
  assert_ne!(source_index, existing_index);
  assert_eq!(map.get_source(source_index).unwrap(), "entry.css");
  assert_eq!(map.get_source_content(source_index).unwrap(), css);
  assert_eq!(map.get_source_content(existing_index).unwrap(), ".existing {}");
}

#[test]
fn uses_the_output_maps_source_index() {
  assert_bundled_source(".entry { color: red }");
}

#[test]
fn registers_css_when_the_inline_map_is_unreadable() {
  assert_bundled_source(".entry { color: red }\n/*# sourceMappingURL=data:application/json,not-a-map */");
}
