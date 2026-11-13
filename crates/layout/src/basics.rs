use macroquad::prelude::*;

#[macroquad::main("Layout Basics")]
async fn main() {
    loop {
        clear_background(WHITE);

        let parent_x = 100.0;
        let parent_y = 100.0;
        let parent_width = 200.0;
        let parent_height = 150.0;
    
    draw_rectangle(
        parent_x, 
        parent_y, 
        parent_width, 
        parent_height, 
        BLUE
    );
    }
}
