use comfy_table::{presets::UTF8_FULL_CONDENSED, ContentArrangement, Table};

use super::CommandOutput;

pub fn render(output: &CommandOutput) {
    if output.rows.is_empty() {
        println!("(no results)");
        return;
    }
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic);
    if !output.headers.is_empty() {
        table.set_header(&output.headers);
    }
    for row in &output.rows {
        table.add_row(row);
    }
    println!("{table}");
}
