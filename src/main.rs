#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window in release mode (Windows only - Linux GUI apps don't show console by default)

use std::{
    ffi::OsString,
    fs::DirEntry
};

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Image sifter",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);

            Ok(Box::<MyApp>::default())
        }),
    )
}

#[derive(Default)]
struct FileSysNode {
    images: Vec<OsString>,
    children: Vec<Box<FileSysNode>>,
    name: OsString,
}

#[derive(Default)]
struct MyApp {
    working_path: Option<OsString>,
    images: Option<Box<FileSysNode>>,
    image_paths: Vec<std::path::PathBuf>, // All images in traversal order
    kept_images: Vec<std::path::PathBuf>,
    discarded_count: usize,
    is_loading: bool,
    image_counter: u64, // Counter to make unique image URIs
    texture: Option<egui::TextureHandle>, // Holds the current image texture
}


fn insert_children(parent: &mut FileSysNode, dir_entry: &DirEntry) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(entries) = dir_entry.path().read_dir() {
        for entry in entries {
            if let Ok(entry) = entry {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_dir() {
                        // Create a new child node for the directory
                        let mut child_node = FileSysNode {
                            name: entry.file_name(),
                            ..FileSysNode::default()
                        };
                        
                        // Recursively populate the child node
                        insert_children(&mut child_node, &entry)?;
                        
                        // Add the child to the parent
                        parent.children.push(Box::new(child_node));
                    } else {
                        // Check if it's an image file before adding
                        if let Some(extension) = entry.path().extension() {
                            if let Some(ext_str) = extension.to_str() {
                                let ext_lower = ext_str.to_lowercase();
                                if matches!(ext_lower.as_str(), "jpg" | "jpeg") {
                                    parent.images.push(entry.file_name());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

impl FileSysNode {
    fn count_images(&self) -> usize {
        let mut count = self.images.len();
        for child in &self.children {
            count += child.count_images();
        }
        count
    }
    
    
    fn get_images_depth_first_current_priority(&self, base_path: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut all_images = Vec::new();
        
        // First, add all images from the current directory
        for image in &self.images {
            let image_path = base_path.join(image);
            all_images.push(image_path);
        }
        
        // Then, recursively add images from subdirectories (depth-first)
        for child in &self.children {
            let child_path = base_path.join(&child.name);
            all_images.extend(child.get_images_depth_first_current_priority(&child_path));
        }
        
        all_images
    }
}

impl MyApp {
    fn copy_kept_images(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(working_path) = &self.working_path {
            let working_path = std::path::PathBuf::from(working_path);
            let output_folder = working_path.join("kept_images");
            
            // Create the output folder if it doesn't exist
            std::fs::create_dir_all(&output_folder)?;
            
            for kept_image_path in &self.kept_images {
                // Calculate relative path from working directory
                let relative_path = kept_image_path.strip_prefix(&working_path)?;
                let destination_path = output_folder.join(relative_path);
                
                // Create parent directories if they don't exist
                if let Some(parent) = destination_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                
                // Copy the JPEG file
                std::fs::copy(kept_image_path, &destination_path)?;
                
                // Check for corresponding CR3 (Canon RAW) file and copy it too
                if let Some(stem) = kept_image_path.file_stem() {
                    let cr3_path = kept_image_path.with_file_name(format!("{}.CR3", stem.to_string_lossy()));
                    let cr3_path_lower = kept_image_path.with_file_name(format!("{}.cr3", stem.to_string_lossy()));
                    
                    // Try both uppercase and lowercase CR3 extensions
                    for potential_cr3 in [&cr3_path, &cr3_path_lower] {
                        if potential_cr3.exists() {
                            let cr3_relative = potential_cr3.strip_prefix(&working_path)?;
                            let cr3_destination = output_folder.join(cr3_relative);
                            
                            // Create parent directories for CR3 if needed
                            if let Some(parent) = cr3_destination.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            
                            // Copy the CR3 file
                            std::fs::copy(potential_cr3, &cr3_destination)?;
                            break; // Only copy one CR3 file if both exist
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if ui.button("Select working folder").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.working_path = Some(path.as_os_str().to_os_string());
                    
                    // Create the root node
                    let mut root_node = FileSysNode {
                        name: path.as_os_str().to_os_string(),
                        ..FileSysNode::default()
                    };

                    // Create a DirEntry-like structure for the root path
                    if let Ok(entries) = path.read_dir() {
                        for entry in entries {
                            if let Ok(entry) = entry {
                                if let Ok(metadata) = entry.metadata() {
                                    if metadata.is_dir() {
                                        // Create child node and recursively populate it
                                        let mut child_node = FileSysNode {
                                            name: entry.file_name(),
                                            ..FileSysNode::default()
                                        };
                                        
                                        if let Err(_e) = insert_children(&mut child_node, &entry) {
                                            // Silently ignore directory processing errors
                                        }
                                        
                                        root_node.children.push(Box::new(child_node));
                                    } else {
                                        // Check if it's an image file before adding to root
                                        if let Some(extension) = entry.path().extension() {
                                            if let Some(ext_str) = extension.to_str() {
                                                let ext_lower = ext_str.to_lowercase();
                                                if matches!(ext_lower.as_str(), "jpg" | "jpeg") {
                                                    root_node.images.push(entry.file_name());
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Silently ignore metadata errors
                                }
                            }
                        }
                    }

                    // Populate the image paths in correct traversal order
                    self.image_paths = root_node.get_images_depth_first_current_priority(&path);
                    
                    self.kept_images.clear();
                    self.discarded_count = 0;
                    self.image_counter = 0;
                    self.is_loading = true;
                    
                    self.images = Some(Box::new(root_node));
                    
                    self.is_loading = false;
                }
            }

            if let Some(picked_path) = &self.working_path {
                ui.horizontal(|ui| {
                    ui.label("Picked folder:");
                    ui.monospace(picked_path.to_string_lossy().as_ref());
                });
                
                // Display information about found images
                if let Some(images_node) = &self.images {
                    let total_images = images_node.count_images();
                    ui.label(format!("Total images found: {} (Current queue: {})", total_images, self.image_paths.len()));
                    
                }
            }

            // Image viewer section
            if !self.image_paths.is_empty() {
                
                // Handle keyboard input
                let mut should_advance = false;
                let mut keep_image = false;
                
                ctx.input(|i| {
                    if i.key_pressed(egui::Key::ArrowRight) {
                        // Keep current image and move to next
                        if !self.image_paths.is_empty() {
                            should_advance = true;
                            keep_image = true;
                        }
                    }
                    if i.key_pressed(egui::Key::ArrowLeft) {
                        // Discard current image and move to next
                        if !self.image_paths.is_empty() {
                            should_advance = true;
                            keep_image = false;
                        }
                    }
                });



                // Current image display
                if !self.image_paths.is_empty() {
                    let current_image_path = &self.image_paths[0];
                    
                    // Progress bar - calculate based on total processed vs original total
                    let total_processed = self.kept_images.len() + self.discarded_count;
                    let original_total = total_processed + self.image_paths.len();
                    let progress = if original_total > 0 { 
                        total_processed as f32 / original_total as f32 
                    } else { 
                        0.0 
                    };
                    ui.add(egui::ProgressBar::new(progress).text(format!("{} / {}", total_processed, original_total)));
                    
                    ui.horizontal(|ui| {
                        ui.label("📷 Current image:");
                        ui.monospace(current_image_path.file_name().unwrap_or_default().to_string_lossy());
                    });

                    // Show statistics
                    ui.horizontal(|ui| {
                        ui.label(format!("✅ Kept: {}", self.kept_images.len()));
                        ui.separator();
                        ui.label(format!("❌ Discarded: {}", self.discarded_count));
                        ui.separator();
                        ui.label(format!("📁 Remaining: {}", self.image_paths.len()));
                    });

                    ui.separator();

                    // Get image bytes (load on demand)
                    let current_image_path_clone = current_image_path.clone();
                    let bytes_uri = format!("bytes://{}/{}", self.image_counter, current_image_path.display());
                    let mut create_texture = false;
                    if self.texture.is_none() {
                        create_texture = true;
                    }
                    if create_texture {
                        let extension = current_image_path_clone.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase());
                        let image_bytes = match std::fs::read(&current_image_path_clone) {
                            Ok(bytes) => Some(bytes),
                            Err(_) => None
                        };
                        if let Some(bytes) = &image_bytes {
                            let color_image = if let Some(ext) = &extension {
                                if ext == "jpg" || ext == "jpeg" {
                                    // Use jpeg-decoder for JPEGs
                                    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
                                    match decoder.decode() {
                                        Ok(decoded) => {
                                            if let Some(info) = decoder.info() {
                                                let width = info.width as usize;
                                                let height = info.height as usize;
                                                let pixels: Vec<egui::Color32> = decoded
                                                    .chunks(3)
                                                    .map(|chunk| egui::Color32::from_rgb(chunk[0], chunk[1], chunk[2]))
                                                    .collect();
                                                egui::ColorImage {
                                                    size: [width, height],
                                                    source_size: egui::Vec2::new(width as f32, height as f32),
                                                    pixels,
                                                }
                                            } else {
                                                egui::ColorImage {
                                                    size: [1, 1],
                                                    source_size: egui::Vec2::new(1.0, 1.0),
                                                    pixels: vec![egui::Color32::BLACK],
                                                }
                                            }
                                        },
                                        Err(_) => egui::ColorImage {
                                            size: [1, 1],
                                            source_size: egui::Vec2::new(1.0, 1.0),
                                            pixels: vec![egui::Color32::BLACK],
                                        },
                                    }
                                } else {
                                    // Use image crate for other formats
                                    match image::load_from_memory(bytes) {
                                        Ok(img) => {
                                            let rgba = img.to_rgba8();
                                            let size = [rgba.width() as usize, rgba.height() as usize];
                                            let pixels = rgba.into_raw();
                                            egui::ColorImage::from_rgba_unmultiplied(size, &pixels)
                                        },
                                        Err(_) => egui::ColorImage {
                                            size: [1, 1],
                                            source_size: egui::Vec2::new(1.0, 1.0),
                                            pixels: vec![egui::Color32::BLACK],
                                        },
                                    }
                                }
                            } else {
                                egui::ColorImage {
                                    size: [1, 1],
                                    source_size: egui::Vec2::new(1.0, 1.0),
                                    pixels: vec![egui::Color32::BLACK],
                                }
                            };
                            self.texture = Some(ctx.load_texture(bytes_uri.clone(), color_image, egui::TextureOptions::default()));
                        } else {
                            self.texture = None;
                        }
                    }

                    // Button click state (also used for keyboard input)
                    // let mut should_advance = false;  // Already declared above for keyboard
                    // let mut keep_image = false;      // Already declared above for keyboard
                    
                    // Use bottom-up layout to reserve space for buttons first
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                        // First place the buttons at the bottom
                        ui.allocate_ui_with_layout(
                            egui::Vec2::new(ui.available_width(), 70.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                let total_button_width = 150.0 + 30.0 + 150.0; // button + space + button
                                let left_space = (ui.available_width() - total_button_width) / 2.0;
                                ui.add_space(left_space);
                                
                                if ui.add_sized([150.0, 60.0], egui::Button::new("👍 Keep")).clicked() {
                                    should_advance = true;
                                    keep_image = true;
                                }
                                
                                ui.add_space(30.0); // Space between buttons
                                
                                if ui.add_sized([150.0, 60.0], egui::Button::new("👎 Discard")).clicked() {
                                    should_advance = true;
                                    keep_image = false;
                                }
                            },
                        );
                        
                        ui.add_space(10.0); // Small gap above buttons
                        
                        // Now use all remaining space for the image
                        ui.vertical_centered(|ui| {
                            if let Some(texture) = &self.texture {
                                ui.add(
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(egui::Vec2::new(
                                            ui.available_width() - 20.0,
                                            ui.available_height()
                                        ))
                                );
                            } else {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Loading image...");
                                });
                            }
                        });
                    });
                    
                    // Handle the action after the UI
                    if should_advance {
                        if keep_image {
                            self.kept_images.push(current_image_path_clone.clone());
                        } else {
                            self.discarded_count += 1;
                        }
                        self.image_paths.remove(0);
                        // Drop the previous texture
                        self.texture = None;
                        self.image_counter += 1;
                        ctx.request_repaint();
                    }

                } else {
                    // All images processed
                    ui.label("🎉 All images processed!");
                    ui.horizontal(|ui| {
                        ui.label(format!("Kept: {}", self.kept_images.len()));
                        ui.label(format!("Discarded: {}", self.discarded_count));
                    });
                    
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        if ui.button("📁 Copy Kept Images").clicked() {
                            match self.copy_kept_images() {
                                Ok(()) => {
                                    // Show success message (you could add a toast notification here)
                                    if let Some(working_path) = &self.working_path {
                                        let output_folder = std::path::PathBuf::from(working_path).join("kept_images");
                                        ui.label(format!("✅ {} images copied to: {}", 
                                            self.kept_images.len(), 
                                            output_folder.display()));
                                    }
                                },
                                Err(e) => {
                                    ui.label(format!("❌ Error copying images: {}", e));
                                }
                            }
                        }
                        
                        if ui.button("🔄 Reset").clicked() {
                            // Rebuild image paths from the filesystem tree
                            if let (Some(images_node), Some(working_path)) = (&self.images, &self.working_path) {
                                let path = std::path::PathBuf::from(working_path);
                                self.image_paths = images_node.get_images_depth_first_current_priority(&path);
                            }
                            
                            self.kept_images.clear();
                            self.discarded_count = 0;
                            self.image_counter = 0;
                        }
                    });
                }
            }
        });
    }
}
