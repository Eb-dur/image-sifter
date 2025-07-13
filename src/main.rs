#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window in release mode (Windows only - Linux GUI apps don't show console by default)

use std::{
    collections::HashMap,
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
    working_index: usize,
    working_path: Option<OsString>,
    images: Option<Box<FileSysNode>>,
    image_paths: Vec<std::path::PathBuf>, // All images in traversal order
    kept_images: Vec<std::path::PathBuf>,
    discarded_images: Vec<std::path::PathBuf>,
    // Image preloading cache
    image_cache: HashMap<usize, Vec<u8>>, // index -> image bytes
    is_loading: bool,
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
    fn preload_images(&mut self) {
        // Preload the next 10 images starting from current index
        let start_index = self.working_index;
        let end_index = (start_index + 10).min(self.image_paths.len());
        
        for i in start_index..end_index {
            // Only load if not already cached
            if !self.image_cache.contains_key(&i) {
                if let Ok(image_bytes) = std::fs::read(&self.image_paths[i]) {
                    self.image_cache.insert(i, image_bytes);
                }
            }
        }
        
        // Aggressively remove images that are outside our cache window
        // Keep only images in range [current_index - 5, current_index + 15]
        let cache_start = self.working_index.saturating_sub(5);
        let cache_end = (self.working_index + 15).min(self.image_paths.len());
        
        let indices_to_remove: Vec<usize> = self.image_cache
            .keys()
            .filter(|&&k| k < cache_start || k >= cache_end)
            .copied()
            .collect();
            
        for index in indices_to_remove {
            self.image_cache.remove(&index);
        }
    }
    
    fn get_image_bytes(&self, index: usize) -> Option<&Vec<u8>> {
        self.image_cache.get(&index)
    }
    
    fn preload_batch_initial(&mut self) {
        // More conservative initial batch - only preload first 5 images
        let end_index = 5.min(self.image_paths.len());
        
        for i in 0..end_index {
            if !self.image_cache.contains_key(&i) {
                if let Ok(image_bytes) = std::fs::read(&self.image_paths[i]) {
                    self.image_cache.insert(i, image_bytes);
                }
            }
        }
    }
    
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
                    
                    self.working_index = 0;
                    self.kept_images.clear();
                    self.discarded_images.clear();
                    self.image_cache.clear();
                    self.is_loading = true;
                    
                    self.images = Some(Box::new(root_node));
                    
                    // Start aggressive preloading of the first batch of images
                    self.preload_batch_initial();
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
                    ui.label(format!("Total images found: {}", total_images));
                    
                }
            }

            // Image viewer section
            if !self.image_paths.is_empty() {
                
                // Handle keyboard input
                let mut advanced = false;
                ctx.input(|i| {
                    if i.key_pressed(egui::Key::ArrowRight) {
                        // Keep current image and move to next
                        if self.working_index < self.image_paths.len() {
                            self.kept_images.push(self.image_paths[self.working_index].clone());
                            self.image_cache.remove(&self.working_index);
                            self.working_index += 1;
                            advanced = true;
                        }
                    }
                    if i.key_pressed(egui::Key::ArrowLeft) {
                        // Discard current image and move to next
                        if self.working_index < self.image_paths.len() {
                            self.discarded_images.push(self.image_paths[self.working_index].clone());
                            self.image_cache.remove(&self.working_index);
                            self.working_index += 1;
                            advanced = true;
                        }
                    }
                });

                // Trigger preloading after input handling
                if advanced {
                    self.preload_images();
                    ctx.request_repaint(); // Immediate repaint for responsiveness
                }

                // Current image display
                if self.working_index < self.image_paths.len() {
                    let current_image_path = &self.image_paths[self.working_index];
                    
                    // Progress bar
                    let progress = (self.working_index as f32) / (self.image_paths.len() as f32);
                    ui.add(egui::ProgressBar::new(progress).text(format!("{} / {}", self.working_index + 1, self.image_paths.len())));
                    
                    ui.horizontal(|ui| {
                        ui.label("📷 Current image:");
                        ui.monospace(current_image_path.file_name().unwrap_or_default().to_string_lossy());
                    });

                    // Show statistics
                    ui.horizontal(|ui| {
                        ui.label(format!("✅ Kept: {}", self.kept_images.len()));
                        ui.separator();
                        ui.label(format!("❌ Discarded: {}", self.discarded_images.len()));
                        ui.separator();
                        ui.label(format!("📁 Remaining: {}", self.image_paths.len() - self.working_index));
                        ui.separator();
                        ui.label(format!("🔄 Cached: {}", self.image_cache.len()));
                    });

                    ui.separator();

                    // Get image bytes first (outside closures to avoid borrowing issues)
                    let cached_bytes = self.get_image_bytes(self.working_index);
                    let current_image_path_clone = current_image_path.clone();
                    
                    // Load image if not cached
                    let image_bytes = if cached_bytes.is_some() {
                        cached_bytes.cloned()
                    } else {
                        match std::fs::read(&current_image_path_clone) {
                            Ok(bytes) => {
                                // Cache this image for future use
                                self.image_cache.insert(self.working_index, bytes.clone());
                                Some(bytes)
                            },
                            Err(_) => None
                        }
                    };

                    // Button click state
                    let mut should_advance = false;
                    let mut keep_image = false;
                    
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
                            if let Some(bytes) = &image_bytes {
                                // Use all available remaining space for the image
                                let bytes_uri = format!("bytes://image_{}", self.working_index);
                                ui.add(
                                    egui::Image::from_bytes(bytes_uri, bytes.clone())
                                        .max_height(ui.available_height())
                                        .max_width(ui.available_width() - 20.0) // Small margin
                                        .fit_to_exact_size(egui::Vec2::new(
                                            ui.available_width() - 20.0,
                                            ui.available_height()
                                        ))
                                );
                            } else {
                                // Show loading indicator or error
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Loading image...");
                                });
                            }
                        });
                    });
                    
                    // Handle the action after the UI
                    if should_advance {
                        let current_index = self.working_index;
                        if keep_image {
                            self.kept_images.push(current_image_path_clone);
                        } else {
                            self.discarded_images.push(current_image_path_clone);
                        }
                        self.image_cache.remove(&current_index);
                        self.working_index += 1;
                        self.preload_images();
                        ctx.request_repaint(); // Immediate repaint for responsiveness
                    }

                } else {
                    // All images processed
                    ui.label("🎉 All images processed!");
                    ui.horizontal(|ui| {
                        ui.label(format!("Kept: {}", self.kept_images.len()));
                        ui.label(format!("Discarded: {}", self.discarded_images.len()));
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
                            self.working_index = 0;
                            self.kept_images.clear();
                            self.discarded_images.clear();
                            self.image_cache.clear();
                            // Restart preloading from the beginning
                            self.preload_images();
                        }
                    });
                }
            }
        });
    }
}
